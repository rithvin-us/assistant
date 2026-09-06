/**
 * TypeScript mirror of `crates/assistant-protocol`.
 *
 * This file is hand-maintained. It is small enough that codegen would cost more
 * than it saves today; if it grows past a page, replace it with `ts-rs` output
 * rather than letting it drift. `PROTOCOL_VERSION` is checked at runtime, so a
 * mismatch surfaces as a visible error rather than a silent misparse.
 */

/** Must equal `assistant_protocol::PROTOCOL_VERSION`. */
export const PROTOCOL_VERSION = 3;

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
