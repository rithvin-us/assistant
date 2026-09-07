//! Conversation WebSocket.
//!
//! Transport glue only. This module reads frames off a socket, builds a
//! [`TurnRequest`], hands it to the orchestrator, and writes the resulting
//! events back out. It contains no orchestration logic: no model call, no tool
//! loop, no permission decision. Those live in `assistant-core`, which does not
//! know this file exists.

use assistant_auth::Principal;
use assistant_core::{
    DomainEvent,
    actions::{ApprovalId, ResolutionOutcome},
    turn::TurnRequest,
};
use assistant_protocol::{
    ApiError, ApprovalOutcome, ClientFrame, PROTOCOL_VERSION, ServerFrame, VoiceState, VoiceTurnId,
};
use assistant_tools::ApprovalStatus;
use axum::{
    Extension,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{orchestration, state::SharedState};

pub async fn stream(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(conversation_id): Path<Uuid>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| handle(socket, state, principal, conversation_id))
}

#[tracing::instrument(skip(socket, state, principal), fields(user_id = %principal.user_id))]
async fn handle(
    mut socket: WebSocket,
    state: SharedState,
    principal: Principal,
    conversation_id: Uuid,
) {
    state
        .events
        .publish(DomainEvent::ConversationOpened { conversation_id });

    // Cancelling this token stops whatever turn is in flight. It is the seam a
    // future voice client uses for barge-in: interrupting playback cancels the
    // turn rather than waiting for it to finish.
    let socket_cancel = CancellationToken::new();

    // The voice turn currently allowed to speak, and the token that stops it.
    //
    // Frames carrying any other turn id are ignored: a transcript or an audio
    // chunk from a turn the user has already abandoned must never reach the
    // client, or stale audio plays over a new question. Barge-in cancels this
    // token, which propagates into the model call and the provider requests.
    let mut voice_turn: Option<(VoiceTurnId, CancellationToken)> = None;

    let mut closed_by_peer = false;

    if send(
        &mut socket,
        ServerFrame::Ready {
            conversation_id,
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    while let Some(Ok(message)) = socket.recv().await {
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => {
                // Echo the close, then let the stream drain so the frame is
                // actually flushed before the socket drops. Dropping straight
                // after `send` can end the TCP connection first, which the peer
                // reports as an abnormal 1006 rather than a clean shutdown.
                let _ = socket.send(Message::Close(None)).await;
                while socket.recv().await.is_some() {}
                closed_by_peer = true;
                break;
            }
            // Binary frames are reserved for audio; nothing consumes them yet.
            _ => continue,
        };

        let frame = match serde_json::from_str::<ClientFrame>(&text) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::debug!(%error, "client sent an unparseable frame");
                let _ = send(
                    &mut socket,
                    ServerFrame::Error(ApiError {
                        code: "bad_frame".into(),
                        message: "frame did not match the client protocol".into(),
                    }),
                )
                .await;
                continue;
            }
        };

        match frame {
            ClientFrame::Ping => {
                if send(&mut socket, ServerFrame::Pong).await.is_err() {
                    break;
                }
            }
            ClientFrame::UserText { text } => {
                let request = TurnRequest::new(conversation_id, principal.clone(), text);

                if run_turn(&mut socket, &state, request, socket_cancel.child_token())
                    .await
                    .is_err()
                {
                    break;
                }
            }

            ClientFrame::ListPendingApprovals => {
                let frame = pending_approvals(&state, &principal).await;
                if send(&mut socket, frame).await.is_err() {
                    break;
                }
            }

            // The client supplies an id and nothing else. It cannot name a tool,
            // pass arguments, assert a risk, or claim to be another user: the
            // server loads the persisted action, scoped to the authenticated
            // principal, and that record decides what happens. See ADR-0015.
            ClientFrame::ApproveAction { approval_id } => {
                let frame = resolve_approval(
                    &state,
                    &principal,
                    approval_id,
                    ApprovalStatus::Approved,
                    socket_cancel.child_token(),
                )
                .await;
                if send(&mut socket, frame).await.is_err() {
                    break;
                }
            }

            ClientFrame::RejectAction { approval_id } => {
                let frame = resolve_approval(
                    &state,
                    &principal,
                    approval_id,
                    ApprovalStatus::Rejected,
                    socket_cancel.child_token(),
                )
                .await;
                if send(&mut socket, frame).await.is_err() {
                    break;
                }
            }

            ClientFrame::VoiceStart { turn_id } => {
                // Starting a turn supersedes any earlier one. Cancel it rather
                // than leaving it running to deliver a late answer.
                if let Some((_, previous)) =
                    voice_turn.replace((turn_id, socket_cancel.child_token()))
                {
                    previous.cancel();
                }
                let _ = send(
                    &mut socket,
                    ServerFrame::VoiceStateChanged {
                        turn_id,
                        state: VoiceState::Listening,
                    },
                )
                .await;
            }

            ClientFrame::VoiceCancel { turn_id } | ClientFrame::VoiceInterrupted { turn_id } => {
                // Only the owner of the active turn may cancel it.
                match &voice_turn {
                    Some((active, cancel)) if *active == turn_id => {
                        // This is the barge-in seam: cancelling propagates into
                        // the model call and the provider requests, so an
                        // interrupted turn stops producing rather than merely
                        // being ignored.
                        cancel.cancel();
                        voice_turn = None;
                        let _ = send(
                            &mut socket,
                            ServerFrame::VoiceStateChanged {
                                turn_id,
                                state: VoiceState::Interrupted,
                            },
                        )
                        .await;
                    }
                    _ => {
                        // A cancel for a turn that is already over. Nothing to
                        // stop, and nothing to tell the client about.
                    }
                }
            }

            ClientFrame::VoiceAudioChunk {
                turn_id,
                data_base64,
                encoding,
            } => {
                // Adopt the turn if the client never sent VoiceStart, but never
                // let a chunk from a superseded turn resurrect it.
                let turn_cancel = match &voice_turn {
                    Some((active, cancel)) if *active == turn_id => cancel.clone(),
                    Some(_) => continue,
                    None => {
                        let cancel = socket_cancel.child_token();
                        voice_turn = Some((turn_id, cancel.clone()));
                        cancel
                    }
                };
                if turn_cancel.is_cancelled() {
                    continue;
                }

                use base64::Engine;
                let audio_bytes =
                    match base64::engine::general_purpose::STANDARD.decode(&data_base64) {
                        Ok(b) => b,
                        Err(_) => continue,
                    };

                let enc = if encoding.contains("mp3") {
                    assistant_voice::AudioEncoding::Mp3
                } else if encoding.contains("webm") {
                    assistant_voice::AudioEncoding::Webm
                } else {
                    assistant_voice::AudioEncoding::Wav
                };

                let _ = send(
                    &mut socket,
                    ServerFrame::VoiceStateChanged {
                        turn_id,
                        state: VoiceState::Transcribing,
                    },
                )
                .await;

                let transcript_text = if let Some(key) = &state.cartesia_api_key {
                    let stt = assistant_voice::CartesiaSttProvider::new(
                        key.clone(),
                        Some(state.cartesia_stt_model.clone()),
                    );
                    let payload = assistant_voice::AudioPayload {
                        bytes: audio_bytes,
                        encoding: enc,
                        sample_rate: 24000,
                        channels: 1,
                    };
                    use assistant_voice::SpeechToTextProvider;
                    match stt.transcribe(payload).await {
                        Ok(res) => res.text,
                        Err(e) => {
                            let _ = send(
                                &mut socket,
                                ServerFrame::Error(ApiError {
                                    code: e.code().into(),
                                    message: e.to_string(),
                                }),
                            )
                            .await;
                            continue;
                        }
                    }
                } else {
                    // Previously this substituted the invented utterance "What
                    // should I do today?" and ran a real turn on it -- tools and
                    // all. Saying nothing was heard is the only honest answer
                    // when there is no transcription provider.
                    let _ = send(
                        &mut socket,
                        ServerFrame::Error(ApiError {
                            code: "stt_unconfigured".into(),
                            message: "Speech-to-text is not configured on the server, so nothing was transcribed.".into(),
                        }),
                    )
                    .await;
                    let _ = send(
                        &mut socket,
                        ServerFrame::VoiceStateChanged {
                            turn_id,
                            state: VoiceState::Error,
                        },
                    )
                    .await;
                    continue;
                };

                let _ = send(
                    &mut socket,
                    ServerFrame::VoiceTranscriptFinal {
                        turn_id,
                        text: transcript_text.clone(),
                    },
                )
                .await;

                let _ = send(
                    &mut socket,
                    ServerFrame::VoiceStateChanged {
                        turn_id,
                        state: VoiceState::Thinking,
                    },
                )
                .await;

                let request = TurnRequest::new(conversation_id, principal.clone(), transcript_text);

                let answer = match run_turn(&mut socket, &state, request, turn_cancel.clone()).await
                {
                    Ok(text) => text,
                    Err(_) => break,
                };

                // Nothing to say is not the same as speaking a filler line.
                let spoken = answer.trim();
                if spoken.is_empty() {
                    // The turn is over; retire it so a late chunk cannot revive it.
                    voice_turn = None;
                    let _ = send(&mut socket, ServerFrame::VoiceEnd { turn_id }).await;
                    let _ = send(
                        &mut socket,
                        ServerFrame::VoiceStateChanged {
                            turn_id,
                            state: VoiceState::Idle,
                        },
                    )
                    .await;
                    continue;
                }

                let _ = send(
                    &mut socket,
                    ServerFrame::VoiceStateChanged {
                        turn_id,
                        state: VoiceState::Speaking,
                    },
                )
                .await;

                // Synthesize TTS for completed answer if Cartesia is configured
                if let Some(key) = &state.cartesia_api_key {
                    let tts = assistant_voice::CartesiaTtsProvider::new(
                        key.clone(),
                        Some(state.cartesia_tts_model.clone()),
                        Some(state.cartesia_tts_voice_id.clone()),
                    );
                    let req = assistant_voice::TtsRequest {
                        // The assistant's actual answer. This was a hardcoded
                        // sentence, so the user heard the same line whatever
                        // they asked and whatever the model replied.
                        text: spoken.to_string(),
                        voice_id: None,
                        model: None,
                        encoding: Some(assistant_voice::AudioEncoding::Wav),
                        sample_rate: Some(24000),
                    };
                    use assistant_voice::TextToSpeechProvider;
                    match tts.synthesize(req).await {
                        Ok(audio) => {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&audio);
                            let _ = send(
                                &mut socket,
                                ServerFrame::VoiceTtsChunk {
                                    turn_id,
                                    audio_base64: b64,
                                    is_final: true,
                                },
                            )
                            .await;
                        }
                        Err(e) => {
                            // A failed synthesis used to be swallowed entirely:
                            // the client saw "speaking" then silence. The answer
                            // is already on screen as text, so say the voice part
                            // failed rather than pretending it played.
                            let _ = send(
                                &mut socket,
                                ServerFrame::Error(ApiError {
                                    code: e.code().into(),
                                    message: e.to_string(),
                                }),
                            )
                            .await;
                        }
                    }
                } else {
                    // No TTS provider. The text answer stands on its own; do not
                    // leave the client waiting for audio that will never arrive.
                    let _ = send(
                        &mut socket,
                        ServerFrame::Error(ApiError {
                            code: "tts_unconfigured".into(),
                            message: "Text-to-speech is not configured on the server, so the reply was not spoken.".into(),
                        }),
                    )
                    .await;
                }

                voice_turn = None;
                let _ = send(&mut socket, ServerFrame::VoiceEnd { turn_id }).await;
                let _ = send(
                    &mut socket,
                    ServerFrame::VoiceStateChanged {
                        turn_id,
                        state: VoiceState::Idle,
                    },
                )
                .await;
            }
        }
    }

    // Anything still running for this socket stops when the client goes away.
    socket_cancel.cancel();

    if !closed_by_peer {
        let _ = socket.send(Message::Close(None)).await;
    }

    state
        .events
        .publish(DomainEvent::ConversationClosed { conversation_id });
}

/// Drives one turn and forwards its events.
///
/// Returns `Err` only when the socket itself failed, so the caller can stop
/// reading. A failed *turn* is delivered as an error frame and the socket stays
/// open, because the user can reasonably try again.
/// Runs one turn, forwarding every frame to the client.
///
/// Returns the assistant's spoken text, accumulated from the deltas as they
/// stream past. The voice path needs the real answer to synthesise: it used to
/// speak a hardcoded sentence because this text was streamed to the client and
/// then dropped on the floor. The text path ignores the return value.
async fn run_turn(
    socket: &mut WebSocket,
    state: &SharedState,
    request: TurnRequest,
    cancel: CancellationToken,
) -> Result<String, axum::Error> {
    let mut events = state.orchestrator.clone().stream(request, cancel.clone());
    let mut answer = String::new();

    while let Some(event) = events.recv().await {
        let Some(frame) = orchestration::to_frame(event) else {
            continue;
        };

        if let ServerFrame::AssistantDelta { text, .. } = &frame {
            answer.push_str(text);
        }

        if send(socket, frame).await.is_err() {
            // The client is gone. Stop the turn rather than letting it run on
            // and spend money producing output nobody will read.
            cancel.cancel();
            return Err(axum::Error::new(std::io::Error::from(
                std::io::ErrorKind::BrokenPipe,
            )));
        }
    }

    Ok(answer)
}

async fn send(socket: &mut WebSocket, frame: ServerFrame) -> Result<(), axum::Error> {
    let text = serde_json::to_string(&frame).expect("ServerFrame is always serialisable");
    socket.send(Message::Text(text.into())).await
}

/// Lists the approvals this principal still has to answer.
async fn pending_approvals(state: &SharedState, principal: &Principal) -> ServerFrame {
    let Some(coordinator) = &state.approvals else {
        return ServerFrame::Error(ApiError {
            code: "no_durable_store".into(),
            message: "this deployment has no database, so approvals are not stored".into(),
        });
    };

    match coordinator
        .store()
        .pending_approvals(principal.user_id)
        .await
    {
        Ok(approvals) => ServerFrame::PendingApprovals {
            approvals: approvals
                .into_iter()
                .map(orchestration::to_pending)
                .collect(),
        },
        Err(error) => {
            tracing::error!(%error, "could not list pending approvals");
            ServerFrame::Error(ApiError {
                code: "internal".into(),
                message: "could not list pending approvals".into(),
            })
        }
    }
}

/// Answers one approval and reports what came of it.
#[tracing::instrument(
    skip(state, principal, cancel),
    fields(user_id = %principal.user_id, approval_id = %approval_id, outcome = ?outcome)
)]
async fn resolve_approval(
    state: &SharedState,
    principal: &Principal,
    approval_id: ApprovalId,
    outcome: ApprovalStatus,
    cancel: CancellationToken,
) -> ServerFrame {
    let Some(coordinator) = &state.approvals else {
        return ServerFrame::Error(ApiError {
            code: "no_durable_store".into(),
            message: "this deployment has no database, so approvals cannot be answered".into(),
        });
    };

    let resolution = coordinator
        .resolve(approval_id, principal, outcome, &cancel)
        .await;

    let outcome = match resolution {
        Ok(ResolutionOutcome::Executed { execution_id, ok }) => {
            ApprovalOutcome::Executed { execution_id, ok }
        }
        Ok(ResolutionOutcome::Rejected { execution_id }) => {
            ApprovalOutcome::Rejected { execution_id }
        }
        Ok(ResolutionOutcome::Expired { .. }) => ApprovalOutcome::Expired,
        Ok(ResolutionOutcome::AlreadyResolved { status }) => ApprovalOutcome::AlreadyResolved {
            status: assistant_core::actions::ApprovalTransitions::as_str(status).to_string(),
        },
        Ok(ResolutionOutcome::NoLongerPermitted {
            execution_id,
            reason,
        }) => ApprovalOutcome::NoLongerPermitted {
            execution_id,
            reason,
        },
        // Not found and not-yours are the same answer on purpose: a caller must
        // not be able to discover that somebody else has a pending approval.
        Err(error) if error.code() == "approval_not_found" => ApprovalOutcome::NotFound,
        Err(error) => {
            tracing::error!(code = error.code(), ?error, "could not resolve approval");
            return ServerFrame::Error(ApiError {
                code: error.code().to_string(),
                message: error.to_string(),
            });
        }
    };

    ServerFrame::ApprovalResolved {
        approval_id,
        outcome,
    }
}
