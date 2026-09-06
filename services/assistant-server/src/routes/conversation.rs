//! Conversation WebSocket.
//!
//! This is the transport that streaming voice and streaming text will both use.
//! No model provider is wired up in this milestone, so a user turn is answered
//! with an explicit "not configured" turn rather than a synthesised reply — the
//! socket proves the streaming path works without pretending to be an assistant.

use assistant_core::DomainEvent;
use assistant_protocol::{ApiError, ClientFrame, PROTOCOL_VERSION, ServerFrame};
use axum::{
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
};
use uuid::Uuid;

use crate::state::SharedState;

pub async fn stream(
    State(state): State<SharedState>,
    Path(conversation_id): Path<Uuid>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |socket| handle(socket, state, conversation_id))
}

#[tracing::instrument(skip(socket, state))]
async fn handle(mut socket: WebSocket, state: SharedState, conversation_id: Uuid) {
    state
        .events
        .publish(DomainEvent::ConversationOpened { conversation_id });

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
            Message::Close(_) => break,
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

        let outgoing = match frame {
            ClientFrame::Ping => vec![ServerFrame::Pong],
            ClientFrame::UserText { .. } => {
                let message_id = Uuid::new_v4();
                vec![
                    ServerFrame::AssistantDelta {
                        message_id,
                        text: "No model provider is configured. ".into(),
                    },
                    ServerFrame::AssistantDelta {
                        message_id,
                        text: "This build only proves the streaming transport.".into(),
                    },
                    ServerFrame::TurnEnd { message_id },
                ]
            }
        };

        for frame in outgoing {
            if send(&mut socket, frame).await.is_err() {
                break;
            }
        }
    }

    state
        .events
        .publish(DomainEvent::ConversationClosed { conversation_id });
}

async fn send(socket: &mut WebSocket, frame: ServerFrame) -> Result<(), axum::Error> {
    let text = serde_json::to_string(&frame).expect("ServerFrame is always serialisable");
    socket.send(Message::Text(text.into())).await
}
