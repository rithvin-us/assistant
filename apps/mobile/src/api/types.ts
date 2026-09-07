/**
 * TypeScript mirror of `crates/assistant-protocol`.
 *
 * This file is hand-maintained. It is small enough that codegen would cost more
 * than it saves today; if it grows past a page, replace it with `ts-rs` output
 * rather than letting it drift. `PROTOCOL_VERSION` is checked at runtime, so a
 * mismatch surfaces as a visible error rather than a silent misparse.
 */

/** Must equal `assistant_protocol::PROTOCOL_VERSION`. */
export const PROTOCOL_VERSION = 7;

export type HealthStatus = "ok" | "degraded";

export interface HealthResponse {
  status: HealthStatus;
  service: string;
  version: string;
  protocol_version: number;
  /** RFC 3339. */
  server_time: string;
}

export interface ApiError {
  code: string;
  message: string;
}

/** Frames the client sends over `WS /v1/conversation/:id/stream`. */
export type ClientFrame =
  | { type: "ping" }
  | { type: "user_text"; text: string }
  /** Ask for the approvals this user still has to answer. */
  | { type: "list_pending_approvals" }
  /**
   * Answer a pending approval.
   *
   * The id is the only thing the client gets to say. It cannot name a tool,
   * supply arguments, assert a risk, or claim to be another user: the server
   * loads the persisted action by id, scoped to the authenticated principal,
   * and that record is authoritative.
   */
  | { type: "approve_action"; approval_id: string }
  | { type: "reject_action"; approval_id: string };

/** One row in the approval sheet. Carries no argument values. */
export interface PendingApproval {
  approval_id: string;
  tool_name: string;
  /** Decided server-side. Display only. */
  risk: RiskLevel;
  reason: string;
  /** The tool and the names of its arguments, never their values. */
  summary: string;
  /** RFC 3339. */
  created_at: string;
  /** RFC 3339. After this the approval can no longer be acted on. */
  expires_at: string;
}

/** What happened when an approval was answered. */
export type ApprovalOutcome =
  | { state: "executed"; execution_id: string; ok: boolean }
  | { state: "rejected"; execution_id: string }
  | { state: "expired" }
  | { state: "already_resolved"; status: string }
  | { state: "no_longer_permitted"; execution_id: string; reason: string }
  | { state: "not_found" };

/**
 * Risk classification of a tool, for display only.
 *
 * The server decides what may run, against its own tool registry. A client must
 * never treat this as authority to execute anything -- it exists so an approval
 * prompt can be shown proportionately.
 */
export type RiskLevel = "green" | "yellow" | "orange" | "red";

/** Frames the server sends back. */
export type ServerFrame =
  | { type: "pong" }
  | { type: "ready"; conversation_id: string; protocol_version: number }
  | { type: "assistant_delta"; message_id: string; text: string }
  /** The assistant asked to use a tool. It has not run and may never run. */
  | { type: "tool_proposed"; call_id: string; name: string; risk: RiskLevel }
  /** The turn stopped, waiting for the user to approve a tool. Nothing ran. */
  | {
      type: "approval_required";
      call_id: string;
      name: string;
      risk: RiskLevel;
      reason: string;
      /**
       * The durable approval to answer. `null` means the server has no database
       * configured, so the action was not persisted and cannot be approved --
       * show it as a notice, not an Approve button.
       */
      approval_id: string | null;
      /** The tool and the names of its arguments, never their values. */
      summary: string;
    }
  | { type: "pending_approvals"; approvals: PendingApproval[] }
  | {
      type: "approval_resolved";
      approval_id: string;
      outcome: ApprovalOutcome;
    }
  /** A tool finished. `ok` is false for a handled failure, which does not end the turn. */
  | { type: "tool_completed"; call_id: string; name: string; ok: boolean }
  | { type: "turn_end"; message_id: string }
  | { type: "error"; code: string; message: string };

/** A user-owned project. Mirrors `assistant_protocol::ProjectItem`. */
export interface ProjectItem {
  id: string;
  user_id: string;
  name: string;
  color: string;
  /**
   * The one project per user that a task with no stated project falls back to,
   * and the one project that cannot be deleted. Flagged on the row rather than
   * matched by name, because the name is the user's to change.
   */
  is_inbox: boolean;
  position: number;
  created_at: string;
  updated_at: string;
}

/** A user-owned label, shared by tasks and notes. Mirrors `LabelItem`. */
export interface LabelItem {
  id: string;
  user_id: string;
  name: string;
  color: string;
  created_at: string;
  updated_at: string;
}

export interface TaskItem {
  id: string;
  user_id: string;
  title: string;
  description: string;
  priority: "P1" | "P2" | "P3" | "P4" | string;
  status: "todo" | "completed" | "archived" | string;
  due_at: string | null;
  project_id: string;
  /**
   * Resolved from `projects.name` by the server. A read projection, not a
   * stored column: to move a task, send `project_id` or `project` on the write
   * rather than mutating this in place.
   */
  project: string;
  /** Resolved label names, sorted. Same projection rule as `project`. */
  labels: string[];
  estimated_minutes?: number | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  /**
   * True while the item is only in the local outbox and has not been
   * acknowledged by the server. Client-side, never sent. See ADR-0029.
   */
  pending?: boolean;
}

export interface ReminderItem {
  id: string;
  user_id: string;
  task_id: string | null;
  title: string;
  remind_at: string;
  status: "pending" | "handled" | "cancelled" | string;
  created_at: string;
  updated_at: string;
  pending?: boolean;
}

export interface NoteItem {
  id: string;
  user_id: string;
  title: string;
  content: string;
  is_archived: boolean;
  /** Resolved label names. Backed by `labels` + `note_labels` since ADR-0028. */
  tags: string[];
  created_at: string;
  updated_at: string;
  pending?: boolean;
}

export interface IdeaItem {
  id: string;
  user_id: string;
  title: string;
  description: string;
  status: "active" | "archived" | "converted" | string;
  converted_task_id: string | null;
  created_at: string;
  updated_at: string;
  pending?: boolean;
}

// ---------------------------------------------------------------------------
// Google Ecosystem & Schedule Foundation Types
// ---------------------------------------------------------------------------

export interface AccountSummary {
  id: string;
  user_id: string;
  provider: string;
  provider_account_id: string;
  email: string;
  display_name?: string | null;
  scopes: string[];
  status: string;
  created_at: string;
  updated_at: string;
}

export interface EmailSummary {
  id: string;
  account_id: string;
  thread_id: string;
  from: string;
  to: string[];
  subject: string;
  date?: string | null;
  snippet: string;
  is_unread: boolean;
}

export interface EmailDetail {
  id: string;
  account_id: string;
  thread_id: string;
  from: string;
  to: string[];
  subject: string;
  date?: string | null;
  body_text: string;
  is_unread: boolean;
}

export interface CalendarEvent {
  id: string;
  account_id: string;
  title: string;
  start_time: string;
  end_time: string;
  description?: string | null;
  location?: string | null;
  all_day: boolean;
}

export interface CreateCalendarEvent {
  account_id: string;
  title: string;
  start_time: string;
  end_time: string;
  description?: string;
  location?: string;
}

export interface UpdateCalendarEvent {
  title?: string;
  start_time?: string;
  end_time?: string;
  description?: string;
  location?: string;
}

export interface FreeSlot {
  start_time: string;
  end_time: string;
  duration_minutes: number;
}

// ---------------------------------------------------------------------------
// Milestone 6 -- academic intelligence
// ---------------------------------------------------------------------------
//
// Mirrors the types at the end of `crates/assistant-protocol/src/lib.rs`.
// Nothing here names Google: the server normalises before it answers.

export type AcademicSource = "manual" | "google_classroom" | "gmail" | "calendar" | "drive";

export interface Course {
  external_id: string;
  account_id: string;
  name: string;
  section?: string | null;
  description?: string | null;
  room?: string | null;
  teacher_name?: string | null;
  state: string;
  alternate_link?: string | null;
  source_updated_at?: string | null;
  /** When the server last heard from the provider. Used to mark stale data. */
  synced_at: string;
}

export interface MaterialRef {
  title?: string | null;
  link?: string | null;
  kind?: string | null;
}

export interface CourseworkItem {
  external_id: string;
  course_external_id: string;
  account_id: string;
  title: string;
  description?: string | null;
  state: string;
  alternate_link?: string | null;
  /** `null` means the assignment has no deadline, never "unknown". */
  due_at?: string | null;
  max_points?: number | null;
  work_type?: string | null;
  materials: MaterialRef[];
  source_updated_at?: string | null;
  synced_at: string;
}

export interface Announcement {
  external_id: string;
  course_external_id: string;
  account_id: string;
  text: string;
  author_name?: string | null;
  alternate_link?: string | null;
  materials: MaterialRef[];
  source_created_at?: string | null;
  source_updated_at?: string | null;
  synced_at: string;
}

export interface DriveFile {
  external_id: string;
  account_id: string;
  name: string;
  mime_type: string;
  size_bytes?: number | null;
  modified_at?: string | null;
  web_view_link?: string | null;
  is_folder: boolean;
  parents: string[];
}

export interface DriveFileContent {
  external_id: string;
  account_id: string;
  name: string;
  mime_type: string;
  text: string;
  /** True when only the leading portion of the file is present. */
  truncated: boolean;
}

export interface AcademicDeadline {
  task_id?: string | null;
  source: AcademicSource;
  external_id?: string | null;
  account_id?: string | null;
  title: string;
  context?: string | null;
  due_at?: string | null;
  is_overdue: boolean;
  is_completed: boolean;
  alternate_link?: string | null;
}

export interface AcademicOverview {
  course_count: number;
  due_this_week: number;
  overdue: number;
  recent_announcement_count: number;
  upcoming: AcademicDeadline[];
  recent_announcements: Announcement[];
  oldest_synced_at?: string | null;
}

export interface AcademicSyncResult {
  courses_synced: number;
  coursework_synced: number;
  announcements_synced: number;
  tasks_created: number;
  tasks_updated: number;
  tasks_skipped_user_edited: number;
}

// ---------------------------------------------------------------------------
// Milestone 7 -- long-term memory
// ---------------------------------------------------------------------------
//
// Mirrors the memory types at the end of `crates/assistant-protocol/src/lib.rs`.
// The model may propose a memory; the server (deterministic Rust) is what
// stores, ranks and updates it. Nothing on the wire lets a client claim
// ownership of somebody else's memory.

export type MemoryKind =
  | "preference"
  | "fact"
  | "idea"
  | "commitment"
  | "project"
  | "temporary";

export type MemoryLifecycle = "active" | "archived" | "superseded";

export type MemorySource =
  | "explicit_user_input"
  | "conversation"
  | "task"
  | "note"
  | "idea"
  | "project"
  | "document"
  | "external_source";

export interface MemoryProvenance {
  source_kind: MemorySource;
  source_ref?: string | null;
}

export interface MemoryItem {
  id: string;
  user_id: string;
  kind: MemoryKind;
  lifecycle: MemoryLifecycle;
  content: string;
  /** 1..=5, higher is more important. */
  importance: number;
  /** 0..=1. Confidence, not importance. */
  confidence: number;
  provenance: MemoryProvenance;
  /** Only meaningful when `kind === "temporary"`. */
  expires_at?: string | null;
  created_at: string;
  updated_at: string;
  last_accessed_at?: string | null;
  access_count: number;
  archived_at?: string | null;
  superseded_by?: string | null;
}

export interface CreateMemoryRequest {
  kind: MemoryKind;
  content: string;
  importance?: number;
  confidence?: number;
  source_kind?: MemorySource;
  source_ref?: string;
  expires_at?: string;
  supersedes?: string;
}

export interface UpdateMemoryRequest {
  kind?: MemoryKind;
  content?: string;
  importance?: number;
  confidence?: number;
  /** Set `null` to clear an expiry; omit to leave alone. */
  expires_at?: string | null;
}

// ---------------------------------------------------------------------------
// Milestone 8 -- document / PDF intelligence
// ---------------------------------------------------------------------------

export type DocumentSource = "local_upload" | "google_drive" | "external_source";

export type DocumentProcessingState =
  | "uploaded"
  | "extracting"
  | "ocr"
  | "verifying"
  | "indexed"
  | "failed";

export type ExtractionMethodDto =
  | "native_text"
  | "ocr"
  | "visual_verification"
  | "none";

export interface DocumentItem {
  id: string;
  user_id: string;
  filename: string;
  mime_type: string;
  size_bytes: number;
  source: DocumentSource;
  source_ref?: string | null;
  content_hash: string;
  page_count?: number | null;
  processing_state: DocumentProcessingState;
  processing_error?: string | null;
  created_at: string;
  updated_at: string;
  processed_at?: string | null;
}

export interface DocumentPageItem {
  document_id: string;
  user_id: string;
  page_number: number;
  extraction_method: ExtractionMethodDto;
  content: string;
  confidence?: number | null;
  char_count: number;
}

export interface DocumentSearchHit {
  document_id: string;
  filename: string;
  page_number: number;
  extraction_method: ExtractionMethodDto;
  snippet: string;
  score: number;
}
