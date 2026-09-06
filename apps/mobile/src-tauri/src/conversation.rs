//! The conversation WebSocket, owned by the shell.
//!
//! React does not open this socket. Network calls, credentials, retries and
//! timeouts live behind `#[tauri::command]` (ADR-0008), and a WebSocket is no
//! exception: the token would otherwise have to be handed to the webview, and
//! the Android and desktop webviews differ on cleartext and mixed-content
//! policy in ways that would show up as a bug on one platform only.
//!
//! Frames are forwarded to the UI verbatim, as the JSON text they arrived as.
//! The shell does not interpret them: `apps/mobile/src/api/types.ts` is the
//! mirror of the wire contract, and having the shell reshape frames on the way
//! past would create a second, undocumented contract to keep in step.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;

/// One frame from the server, as received.
pub const FRAME_EVENT: &str = "conversation://frame";
/// The socket's own lifecycle, which is not part of the wire protocol.
pub const STATUS_EVENT: &str = "conversation://status";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
enum Status {
    Open,
    /// The socket ended. `reason` is a short, user-safe phrase -- never a
    /// transport error string, which can contain the URL and therefore the
    /// access token that is carried in its query string.
    Closed { reason: String },
}

/// The live connection, if there is one.
#[derive(Default)]
pub struct Connection {
    inner: Mutex<Option<Open>>,
}

struct Open {
    outgoing: mpsc::Sender<String>,
    task: tokio::task::JoinHandle<()>,
}

impl Connection {
    /// Connects, replacing any existing connection.
    ///
    /// The token is placed in the query string because a browser-style
    /// WebSocket handshake cannot carry headers; the server logs request paths
    /// without their query string for exactly this reason. Nothing in this
    /// function logs the URL.
    pub async fn open(
        self: &Arc<Self>,
        app: AppHandle,
        base_url: &str,
        token: &str,
        conversation_id: &str,
    ) -> Result<(), String> {
        self.close().await;

        let base = base_url.trim_end_matches('/');
        let scheme = if base.starts_with("https://") {
            "wss://"
        } else {
            "ws://"
        };
        let host = base
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        let url =
            format!("{scheme}{host}/v1/conversation/{conversation_id}/stream?access_token={token}");

        let (socket, _response) = tokio_tungstenite::connect_async(&url)
            .await
            // The error is deliberately discarded: `tungstenite`'s Display can
            // include the URL, and the URL contains the token.
            .map_err(|_| "could not reach the assistant".to_string())?;

        let (mut writer, mut reader) = socket.split();
        let (outgoing, mut to_send) = mpsc::channel::<String>(16);

        let task = tokio::spawn(async move {
            let reason = loop {
                tokio::select! {
                    // Outgoing frames from the UI.
                    message = to_send.recv() => match message {
                        Some(text) => {
                            if writer.send(Message::Text(text.into())).await.is_err() {
                                break "the connection dropped";
                            }
                        }
                        // The sender was dropped: `close` was called.
                        None => {
                            let _ = writer.send(Message::Close(None)).await;
                            break "closed";
                        }
                    },

                    // Incoming frames from the server.
                    message = reader.next() => match message {
                        Some(Ok(Message::Text(text))) => {
                            // Emitting the raw text keeps the protocol mirror
                            // in TypeScript the single place frames are parsed.
                            let _ = app.emit(FRAME_EVENT, text.to_string());
                        }
                        Some(Ok(Message::Close(_))) | None => break "closed",
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break "the connection dropped",
                    },
                }
            };

            let _ = app.emit(
                STATUS_EVENT,
                Status::Closed {
                    reason: reason.to_string(),
                },
            );
        });

        *self.inner.lock().await = Some(Open { outgoing, task });
        Ok(())
    }

    pub async fn send(&self, frame: String) -> Result<(), String> {
        let guard = self.inner.lock().await;
        let Some(open) = guard.as_ref() else {
            return Err("not connected".to_string());
        };
        open.outgoing
            .send(frame)
            .await
            .map_err(|_| "not connected".to_string())
    }

    /// Closes the socket, if one is open.
    ///
    /// Dropping the sender ends the forwarding task, which sends a Close frame.
    /// The server cancels whatever turn was in flight when the socket goes
    /// away, so this is also how a user leaving stops the assistant spending.
    pub async fn close(&self) {
        if let Some(open) = self.inner.lock().await.take() {
            drop(open.outgoing);
            open.task.abort();
        }
    }

    pub async fn is_open(&self) -> bool {
        self.inner.lock().await.is_some()
    }
}

/// Opens the conversation socket.
#[tauri::command]
pub async fn conversation_open(
    app: AppHandle,
    connection: tauri::State<'_, Arc<Connection>>,
    base_url: String,
    token: String,
    conversation_id: String,
) -> Result<(), String> {
    let connection = connection.inner().clone();
    connection
        .open(app.clone(), &base_url, &token, &conversation_id)
        .await?;
    let _ = app.emit(STATUS_EVENT, Status::Open);
    Ok(())
}

/// Sends one already-serialised client frame.
///
/// The shell does not build frames: the TypeScript mirror of the protocol does,
/// so there is one place a protocol change has to be made on this side.
#[tauri::command]
pub async fn conversation_send(
    connection: tauri::State<'_, Arc<Connection>>,
    frame: String,
) -> Result<(), String> {
    connection.send(frame).await
}

#[tauri::command]
pub async fn conversation_close(
    connection: tauri::State<'_, Arc<Connection>>,
) -> Result<(), String> {
    connection.close().await;
    Ok(())
}

#[tauri::command]
pub async fn conversation_is_open(
    connection: tauri::State<'_, Arc<Connection>>,
) -> Result<bool, String> {
    Ok(connection.is_open().await)
}
