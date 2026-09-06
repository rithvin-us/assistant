/**
 * TypeScript mirror of `crates/assistant-protocol`.
 *
 * This file is hand-maintained. It is small enough that codegen would cost more
 * than it saves today; if it grows past a page, replace it with `ts-rs` output
 * rather than letting it drift. `PROTOCOL_VERSION` is checked at runtime, so a
 * mismatch surfaces as a visible error rather than a silent misparse.
 */

/** Must equal `assistant_protocol::PROTOCOL_VERSION`. */
export const PROTOCOL_VERSION = 2;

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
  | { type: "user_text"; text: string };

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
    }
  /** A tool finished. `ok` is false for a handled failure, which does not end the turn. */
  | { type: "tool_completed"; call_id: string; name: string; ok: boolean }
  | { type: "turn_end"; message_id: string }
  | { type: "error"; code: string; message: string };
