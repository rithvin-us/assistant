//! Wire contract between the mobile app and the assistant server.
//!
//! This crate is deliberately dependency-light: it holds only data, no logic and
//! no I/O. Both `assistant-server` and the Tauri shell depend on it so a change
//! to the contract is a compile error on both sides rather than a runtime
//! surprise. The TypeScript mirror lives in `apps/mobile/src/api/types.ts`.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Bumped whenever a breaking change is made to the types in this crate. The
/// client sends the version it was built against so the server can reject a
/// mismatched build instead of misparsing it.
pub const PROTOCOL_VERSION: u32 = 1;

pub type ConversationId = Uuid;
pub type MessageId = Uuid;

/// Response of `GET /v1/health`. Used by the mobile app to prove connectivity
/// and to detect a protocol mismatch before anything else is attempted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub service: String,
    pub version: String,
    pub protocol_version: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub server_time: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

/// Uniform error body for every non-2xx HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    /// Stable machine-readable discriminant, e.g. `unauthorized`.
    pub code: String,
    /// Human-readable message. Safe to show to the user; never contains secrets.
    pub message: String,
}

/// Frames sent by the client over `WS /v1/conversation/:id/stream`.
///
/// The variants below are the minimum needed to prove the transport works.
/// Voice frames (audio chunks, barge-in) and tool-approval frames are future
/// additions; the enum is `#[non_exhaustive]`-shaped by convention so adding
/// them is not a breaking change for the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    /// Liveness probe. The server answers with [`ServerFrame::Pong`].
    Ping,
    /// A user turn as text. Voice turns will arrive as a separate variant.
    UserText { text: String },
}

/// Frames sent by the server over the conversation socket.
///
/// Assistant output is modelled as a stream of deltas terminated by
/// [`ServerFrame::TurnEnd`] so that token streaming and, later, streamed audio
/// need no change to the transport shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Pong,
    /// Sent once when the socket is accepted.
    Ready {
        conversation_id: ConversationId,
        protocol_version: u32,
    },
    /// Incremental assistant output for the current turn.
    AssistantDelta {
        message_id: MessageId,
        text: String,
    },
    /// Terminates the current turn.
    TurnEnd {
        message_id: MessageId,
    },
    /// A recoverable error. The socket stays open.
    Error(ApiError),
}
