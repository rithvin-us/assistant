//! Conversation WebSocket.
//!
//! Transport glue only. This module reads frames off a socket, builds a
//! [`TurnRequest`], hands it to the orchestrator, and writes the resulting
//! events back out. It contains no orchestration logic: no model call, no tool
//! loop, no permission decision. Those live in `assistant-core`, which does not
//! know this file exists.

use assistant_auth::Principal;
use assistant_core::{DomainEvent, turn::TurnRequest};
use assistant_protocol::{ApiError, ClientFrame, PROTOCOL_VERSION, ServerFrame};
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
