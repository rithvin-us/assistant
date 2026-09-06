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
pub const PROTOCOL_VERSION: u32 = 4;

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
    UserText {
        text: String,
    },

    /// Ask for the approvals this user still has to answer.
    ListPendingApprovals,

    /// Answer a pending approval.
    ///
    /// The id is the *only* thing the client gets to say. It cannot name a tool,
    /// supply arguments, assert a risk level, or claim to be a different user:
    /// the server loads the persisted action by id, scoped to the authenticated
    /// principal, and that record is authoritative. See ADR-0015.
    ApproveAction {
        approval_id: ApprovalId,
    },
    RejectAction {
        approval_id: ApprovalId,
    },
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

    /// The assistant asked to use a tool. It has not run and may never run.
    ///
    /// `risk` is the authoritative classification from the server's tool
    /// registry. It is never anything the model supplied, and a client may rely
    /// on that when deciding how prominently to show the call.
    ToolProposed {
        call_id: String,
        name: String,
        risk: RiskLevel,
    },

    /// The turn stopped and is waiting for the user to approve a tool.
    ///
    /// Nothing has run. The client is expected to show an approval prompt; until
    /// a decision arrives the turn stays stopped.
    ApprovalRequired {
        call_id: String,
        name: String,
        risk: RiskLevel,
        reason: String,
        /// The durable approval to answer with [`ClientFrame::ApproveAction`].
        ///
        /// `None` means the server has no durable store configured, so the
        /// action was not persisted and cannot be answered. A client must show
        /// this as an unactionable notice rather than an Approve button.
        approval_id: Option<ApprovalId>,
        /// Sanitised description of the action: the tool and the names of its
        /// arguments, never their values. See ADR-0016.
        summary: String,
    },

    /// The approvals this user still has to answer.
    PendingApprovals {
        approvals: Vec<PendingApproval>,
    },

    /// An approval was answered, and this is what came of it.
    ApprovalResolved {
        approval_id: ApprovalId,
        outcome: ApprovalOutcome,
    },

    /// A tool finished. `ok` distinguishes success from a handled failure; a
    /// failed tool does not end the turn.
    ToolCompleted {
        call_id: String,
        name: String,
        ok: bool,
    },

    /// Terminates the current turn.
    TurnEnd {
        message_id: MessageId,
    },

    /// A recoverable error. The socket stays open.
    Error(ApiError),
}

/// Risk classification of a tool, mirrored on the wire so a client can render
/// an approval prompt proportionately.
///
/// This is a copy of the server-side classification for display only. A client
/// must never treat it as authority to run anything: every decision is made
/// server-side against the tool registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Green,
    Yellow,
    Orange,
    Red,
}

pub type ApprovalId = Uuid;
pub type ExecutionId = Uuid;

/// One row in the approval sheet.
///
/// Carries only what a user needs to decide. Deliberately absent: the argument
/// values, the conversation, and anything about the model. A calendar title is
/// harmless but an email body is not, and the wire format cannot tell them
/// apart -- so it carries neither. See ADR-0016.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub approval_id: ApprovalId,
    /// Namespaced tool identifier, e.g. `gmail.send`.
    pub tool_name: String,
    /// Authoritative risk, decided server-side. Display only -- a client cannot
    /// change what running this action requires by altering this value.
    pub risk: RiskLevel,
    /// Why approval was asked for, written for a human.
    pub reason: String,
    /// The tool and the names of its arguments.
    pub summary: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// After this the approval can no longer be acted on.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

/// What happened when an approval was answered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ApprovalOutcome {
    /// Approved, and the action ran. `ok` is false if the tool itself failed.
    Executed { execution_id: ExecutionId, ok: bool },
    /// Rejected. Nothing ran.
    Rejected { execution_id: ExecutionId },
    /// The window had closed. Nothing ran.
    Expired,
    /// Already answered -- typically a double tap. Nothing ran a second time.
    AlreadyResolved { status: String },
    /// Policy changed while the approval was pending. Nothing ran.
    NoLongerPermitted {
        execution_id: ExecutionId,
        reason: String,
    },
    /// No such approval for this user. Deliberately indistinguishable from an
    /// approval belonging to somebody else.
    NotFound,
}

/// A user-owned project. Tasks reference one by id; see ADR-0028.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: String,
    /// The one project per user that a task with no stated project falls back
    /// to, and the one project that cannot be deleted. Flagged in the row
    /// rather than matched by name, because the name is the user's to change.
    pub is_inbox: bool,
    pub position: i32,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// A user-owned label, shared by tasks and notes; see ADR-0028.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Standalone Task model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub description: String,
    pub priority: String,
    pub status: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub due_at: Option<OffsetDateTime>,
    pub project_id: Uuid,
    /// Resolved from `projects.name` on read. A projection for display, not a
    /// stored column -- writes name the project by `project_id` or by name in
    /// the request body, never by echoing this field back.
    pub project: String,
    /// Resolved label names, sorted. Same projection rule as `project`.
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub estimated_minutes: Option<u32>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

/// Standalone Reminder model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub task_id: Option<Uuid>,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub remind_at: OffsetDateTime,
    pub status: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Standalone Note model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub content: String,
    pub is_archived: bool,
    /// Resolved label names, sorted. Backed by `labels` + `note_labels` since
    /// ADR-0028, not by a stored `text[]`.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Standalone Idea model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdeaItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub description: String,
    pub status: String,
    pub converted_task_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Sanitized summary of a connected external identity (Google account).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSummary {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub provider_account_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub scopes: Vec<String>,
    pub status: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Normalized provider-neutral summary of an email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSummary {
    pub id: String,
    pub account_id: Uuid,
    pub thread_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub snippet: String,
    pub is_unread: bool,
}

/// Normalized provider-neutral full email content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailDetail {
    pub id: String,
    pub account_id: Uuid,
    pub thread_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub body_text: String,
    pub is_unread: bool,
}

/// Normalized provider-neutral calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub account_id: Uuid,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub description: Option<String>,
    pub location: Option<String>,
    pub all_day: bool,
}

/// Input payload to create a new calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEventRequest {
    pub account_id: Uuid,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub description: Option<String>,
    pub location: Option<String>,
}

/// Deterministic available free time slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeSlot {
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub duration_minutes: u32,
}
