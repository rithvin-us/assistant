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
use assistant_protocol::{ApiError, ApprovalOutcome, ClientFrame, PROTOCOL_VERSION, ServerFrame};
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
async fn run_turn(
    socket: &mut WebSocket,
    state: &SharedState,
    request: TurnRequest,
    cancel: CancellationToken,
) -> Result<(), axum::Error> {
    let mut events = state.orchestrator.clone().stream(request, cancel.clone());

    while let Some(event) = events.recv().await {
        let Some(frame) = orchestration::to_frame(event) else {
            continue;
        };

        if send(socket, frame).await.is_err() {
            // The client is gone. Stop the turn rather than letting it run on
            // and spend money producing output nobody will read.
            cancel.cancel();
            return Err(axum::Error::new(std::io::Error::from(
                std::io::ErrorKind::BrokenPipe,
            )));
        }
    }

    Ok(())
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
