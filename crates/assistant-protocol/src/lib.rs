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
pub const PROTOCOL_VERSION: u32 = 7;

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

// ---------------------------------------------------------------------------
// Milestone 6 -- academic intelligence
// ---------------------------------------------------------------------------
//
// These are provider-neutral on purpose. Nothing below names Google, and no
// Google JSON reaches the UI or `assistant-core`: the concrete Classroom and
// Drive clients in `assistant-server` normalise into these shapes. Adding a
// second academic provider later should mean writing another normaliser, not
// changing the wire contract. See ADR-0032.

/// Where a piece of academic information came from. Carried so the UI can say
/// "imported from Classroom" rather than presenting every task as if the user
/// typed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcademicSource {
    Manual,
    GoogleClassroom,
    Gmail,
    Calendar,
    Drive,
}

/// A course the user is enrolled in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Course {
    /// Provider identifier, stable for the life of the course.
    pub external_id: String,
    pub account_id: Uuid,
    pub name: String,
    pub section: Option<String>,
    pub description: Option<String>,
    pub room: Option<String>,
    pub teacher_name: Option<String>,
    /// Provider's own lifecycle state, e.g. `ACTIVE` or `ARCHIVED`.
    pub state: String,
    /// Link into the provider's own UI, when it offers one.
    pub alternate_link: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub source_updated_at: Option<OffsetDateTime>,
    /// When this application last heard from the provider. The UI uses it to
    /// mark data as stale rather than implying a live read.
    #[serde(with = "time::serde::rfc3339")]
    pub synced_at: OffsetDateTime,
}

/// Metadata for a file attached to coursework or an announcement. Deliberately
/// metadata only -- no file body ever travels in this type.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MaterialRef {
    pub title: Option<String>,
    pub link: Option<String>,
    /// e.g. `drive_file`, `link`, `youtube_video`, `form`.
    pub kind: Option<String>,
}

/// A single assignment or question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseworkItem {
    pub external_id: String,
    pub course_external_id: String,
    pub account_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub state: String,
    pub alternate_link: Option<String>,
    /// `None` means the assignment genuinely has no deadline -- never
    /// "we could not work one out". A provider that sends a partial due date
    /// yields `None`, because a guessed deadline is worse than no deadline.
    #[serde(with = "time::serde::rfc3339::option")]
    pub due_at: Option<OffsetDateTime>,
    pub max_points: Option<f64>,
    pub work_type: Option<String>,
    #[serde(default)]
    pub materials: Vec<MaterialRef>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub source_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub synced_at: OffsetDateTime,
}

/// A course announcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub external_id: String,
    pub course_external_id: String,
    pub account_id: Uuid,
    pub text: String,
    pub author_name: Option<String>,
    pub alternate_link: Option<String>,
    #[serde(default)]
    pub materials: Vec<MaterialRef>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub source_created_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub source_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub synced_at: OffsetDateTime,
}

/// A file in the user's cloud storage. Metadata only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveFile {
    pub external_id: String,
    pub account_id: Uuid,
    pub name: String,
    pub mime_type: String,
    /// Absent for files the provider does not report a size for, which
    /// includes native editor documents.
    pub size_bytes: Option<u64>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,
    pub web_view_link: Option<String>,
    pub is_folder: bool,
    /// Parent folder identifiers, as reported by the provider.
    #[serde(default)]
    pub parents: Vec<String>,
}

/// The body of a file small enough and simple enough to read inline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveFileContent {
    pub external_id: String,
    pub account_id: Uuid,
    pub name: String,
    pub mime_type: String,
    pub text: String,
    /// True when the provider gave us more than the configured ceiling and the
    /// text above is the leading portion rather than the whole file. The UI
    /// must say so rather than implying a complete read.
    pub truncated: bool,
}

/// One obligation with a deadline, normalised across sources so the overview
/// and the scheduler can treat Classroom coursework and a manual task the
/// same way.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcademicDeadline {
    /// Set when a task row backs this deadline.
    pub task_id: Option<Uuid>,
    pub source: AcademicSource,
    /// Provider identifier, when the deadline came from one.
    pub external_id: Option<String>,
    pub account_id: Option<Uuid>,
    pub title: String,
    /// Human-facing origin, e.g. the course name.
    pub context: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub due_at: Option<OffsetDateTime>,
    pub is_overdue: bool,
    pub is_completed: bool,
    pub alternate_link: Option<String>,
}

/// The Academic Overview payload. Every number here is counted from rows, not
/// inferred by a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcademicOverview {
    pub course_count: usize,
    pub due_this_week: usize,
    pub overdue: usize,
    pub recent_announcement_count: usize,
    /// Nearest deadlines first.
    pub upcoming: Vec<AcademicDeadline>,
    pub recent_announcements: Vec<Announcement>,
    /// When the oldest contributing cache was last refreshed. `None` when
    /// nothing has ever synced.
    #[serde(with = "time::serde::rfc3339::option")]
    pub oldest_synced_at: Option<OffsetDateTime>,
}

/// Result of importing coursework into tasks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AcademicSyncResult {
    pub courses_synced: usize,
    pub coursework_synced: usize,
    pub announcements_synced: usize,
    pub tasks_created: usize,
    pub tasks_updated: usize,
    /// Imported tasks left alone because the user had edited the field the
    /// provider wanted to change.
    pub tasks_skipped_user_edited: usize,
}

// ---------------------------------------------------------------------------
// Milestone 7 -- long-term memory
// ---------------------------------------------------------------------------
//
// The wire-side mirror of `crates/assistant-memory`. Nothing here changes what
// the server enforces: ownership, secret rejection, lifecycle transitions and
// ranking are decided in Rust. These types exist so the mobile Memory screen
// and any future client speak the same vocabulary as the server.

/// Kinds of memory. Kept in lockstep with `assistant_memory::MemoryKind` and
/// with the database `check` constraint on `memories.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKindDto {
    Preference,
    Fact,
    Idea,
    Commitment,
    Project,
    Temporary,
}

/// Where a memory sits in its life. Same set as `assistant_memory::Lifecycle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycleDto {
    Active,
    Archived,
    Superseded,
}

/// Where a memory came from. Same set as `assistant_memory::MemorySource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceDto {
    ExplicitUserInput,
    Conversation,
    Task,
    Note,
    Idea,
    Project,
    Document,
    ExternalSource,
}

/// Provenance tuple. `source_ref` is `None` for explicit user input, which has
/// no other identifier the client needs to know about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProvenanceDto {
    pub source_kind: MemorySourceDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

/// A memory row as the client sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: MemoryKindDto,
    pub lifecycle: MemoryLifecycleDto,
    pub content: String,
    /// 1..=5, higher is more important. Set by the application, not by the
    /// model.
    pub importance: u8,
    /// 0.0..=1.0. Confidence in the content; not the same as importance.
    pub confidence: f32,
    pub provenance: MemoryProvenanceDto,
    /// Only meaningful for temporary memories.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_accessed_at: Option<OffsetDateTime>,
    pub access_count: u32,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub archived_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<Uuid>,
}

/// Payload for `POST /v1/memories`.
///
/// `user_id` is deliberately absent: the server takes it from the
/// authenticated principal. A client cannot claim to be somebody else, and
/// echoing the id back would just create a shape you could get wrong.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemoryRequest {
    pub kind: MemoryKindDto,
    pub content: String,
    /// Defaults to `3` when absent.
    #[serde(default)]
    pub importance: Option<u8>,
    /// Defaults to `1.0` for the explicit path when absent.
    #[serde(default)]
    pub confidence: Option<f32>,
    /// Defaults to `explicit_user_input` when absent.
    #[serde(default)]
    pub source_kind: Option<MemorySourceDto>,
    #[serde(default)]
    pub source_ref: Option<String>,
    /// Required for `kind == Temporary`.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// The old memory this one replaces. Marked superseded on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<Uuid>,
}

/// Payload for `PATCH /v1/memories/:id`. Every field is optional; the server
/// leaves unnamed fields alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateMemoryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<MemoryKindDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// `Some(Some(_))` sets, `Some(None)` clears, `None` leaves alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Option<OffsetDateTime>>,
}

/// A model- or integration-authored suggestion. Not authoritative: the server
/// validates every field before it becomes a `MemoryItem`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProposalDto {
    pub kind: MemoryKindDto,
    pub content: String,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub importance: Option<u8>,
    #[serde(default)]
    pub reason: Option<String>,
    pub source_kind: MemorySourceDto,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

// ---------------------------------------------------------------------------
// Milestone 8 -- document / PDF intelligence
// ---------------------------------------------------------------------------
//
// Mirrors `assistant_documents`. The mobile Documents screen speaks this
// vocabulary; nothing on the wire lets a client claim ownership of another
// user's document.

/// Where a document was ingested from. Same set as
/// `assistant_documents::DocumentSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSourceDto {
    LocalUpload,
    GoogleDrive,
    ExternalSource,
}

/// The document's processing state. Same set as
/// `assistant_documents::ProcessingState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentProcessingStateDto {
    Uploaded,
    Extracting,
    Ocr,
    Verifying,
    Indexed,
    Failed,
}

/// How this page's text came to be. Same set as
/// `assistant_documents::ExtractionMethod`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMethodDto {
    NativeText,
    Ocr,
    VisualVerification,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentItem {
    pub id: Uuid,
    pub user_id: Uuid,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub source: DocumentSourceDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    pub content_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    pub processing_state: DocumentProcessingStateDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_error: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub processed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPageItem {
    pub document_id: Uuid,
    pub user_id: Uuid,
    pub page_number: u32,
    pub extraction_method: ExtractionMethodDto,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    pub char_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSearchHit {
    pub document_id: Uuid,
    pub filename: String,
    pub page_number: u32,
    pub extraction_method: ExtractionMethodDto,
    pub snippet: String,
    pub score: f32,
}

/// Payload to ingest a Google Drive file by id.
///
/// The server downloads the bytes (using the M6 Drive client), stores them in
/// the object store, and runs the processing pipeline. The client never sends
/// the bytes themselves; that keeps the mobile bundle out of the object-store
/// authentication path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestFromDriveRequest {
    pub account_id: Uuid,
    pub file_id: String,
}
