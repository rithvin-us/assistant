//! The request lifecycle.
//!
//! A turn is one user input and everything the assistant does about it. These
//! types are the vocabulary the orchestrator, the transport and the audit log
//! all share. Nothing here knows about HTTP, WebSockets or Tauri.

use assistant_auth::Principal;
use assistant_tools::{RiskLevel, ToolCall, ToolResult};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Correlates every log line, event and tool call belonging to one turn.
pub type TurnId = Uuid;
pub type ConversationId = Uuid;
pub type MessageId = Uuid;

/// What the caller asks the assistant to do.
///
/// Constructed by the transport layer from whatever arrived on the wire. The
/// orchestrator never reads a socket, a header or an environment variable.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub turn_id: TurnId,
    pub conversation_id: ConversationId,
    /// Who is asking. Supplies the scopes the permission policy checks against.
    pub principal: Principal,
    /// Raw user text, exactly as received. Normalisation happens later and does
    /// not mutate this.
    pub input: String,
    pub received_at: OffsetDateTime,
}

impl TurnRequest {
    pub fn new(
        conversation_id: ConversationId,
        principal: Principal,
        input: impl Into<String>,
    ) -> Self {
        Self {
            turn_id: Uuid::new_v4(),
            conversation_id,
            principal,
            input: input.into(),
            received_at: OffsetDateTime::now_utc(),
        }
    }
}

/// User input after deterministic cleanup.
///
/// Normalisation is trimming and rejection, never interpretation. Anything that
/// requires understanding what the user meant belongs to a later stage, and must
/// not silently rewrite what they said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedInput {
    /// The user's content with surrounding and repeated whitespace collapsed.
    pub text: String,
    /// Lowercased `text`, provided so deterministic matchers do not each
    /// reimplement case folding. Never shown to the user or sent to a model.
    pub matchable: String,
}

/// Deterministic input normalisation.
///
/// Collapses runs of whitespace (including the newlines a speech-to-text stream
/// will eventually produce) and rejects input with no content. It does not
/// truncate, translate, spell-correct or summarise: the user's semantic content
/// survives unchanged.
pub fn normalize(input: &str) -> Result<NormalizedInput, crate::CoreError> {
    let text = input.split_whitespace().collect::<Vec<_>>().join(" ");

    if text.is_empty() {
        return Err(crate::CoreError::InvalidInput);
    }

    Ok(NormalizedInput {
        matchable: text.to_lowercase(),
        text,
    })
}

/// Context gathered for this turn. Deliberately small: it grows when a real
/// context source exists to fill it, not before.
#[derive(Debug, Clone, Default)]
pub struct TurnContext {
    /// Prior conversation turns, oldest first.
    pub history: Vec<ContextMessage>,
    /// Facts a provider considered relevant. Free-form for now because no
    /// retrieval system exists yet to give them structure.
    pub facts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    pub role: ContextRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRole {
    User,
    Assistant,
}

/// How the orchestrator decided to answer.
///
/// Chosen by deterministic routing code before any model is contacted. The model
/// is never asked whether it should have been used -- that question answers
/// itself too late and costs a round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Answered from code and local state. No model invocation at all.
    Deterministic,
    /// The model answers, and no tools are offered to it.
    Model,
    /// The model answers and may propose tool calls.
    ModelWithTools,
}

/// The routing decision plus what produced it.
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub mode: ExecutionMode,
    /// Name of the deterministic handler that claimed this turn, when
    /// `mode == Deterministic`. Recorded so logs can say *why* a turn skipped
    /// the model.
    pub handler: Option<String>,
}

/// Everything a completed turn produced.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub mode: ExecutionMode,
    /// The assistant's final text.
    pub text: String,
    /// Tool calls the model proposed, in proposal order, including ones that
    /// were denied or never ran.
    pub proposed: Vec<ToolCall>,
    /// Results of the calls that actually executed.
    pub executed: Vec<ToolResult>,
    pub rounds: usize,
}

/// Core-level streaming events.
///
/// This is the orchestrator's output contract. The transport layer translates
/// these into whatever its wire format is; the orchestrator has no idea what
/// that format is. Adding a variant must not require a core change to consumers
/// that do not care about it, so consumers should match non-exhaustively.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnEvent {
    Started {
        turn_id: TurnId,
        mode: ExecutionMode,
    },
    AssistantStarted {
        message_id: MessageId,
    },
    /// One increment of assistant text.
    AssistantDelta {
        message_id: MessageId,
        text: String,
    },
    /// The model asked for a tool. It has not run and may never run.
    ToolProposed {
        call_id: String,
        name: String,
        /// Authoritative risk, read from the registered `ToolSpec` -- never from
        /// anything the model said.
        risk: RiskLevel,
    },
    /// Policy stopped the turn pending a human decision.
    ApprovalRequired {
        call_id: String,
        name: String,
        risk: RiskLevel,
        reason: String,
    },
    ToolStarted {
        call_id: String,
        name: String,
    },
    ToolCompleted {
        call_id: String,
        name: String,
        ok: bool,
    },
    Completed {
        turn_id: TurnId,
        message_id: MessageId,
        mode: ExecutionMode,
        rounds: usize,
    },
    Failed {
        turn_id: TurnId,
        /// Stable discriminant from [`crate::CoreError::code`].
        code: &'static str,
        /// User-safe message. Never contains provider internals.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_collapses_whitespace_without_changing_meaning() {
        let normalized = normalize("  what   is\n\tmy   status?  ").expect("accepted");
        assert_eq!(normalized.text, "what is my status?");
        assert_eq!(normalized.matchable, "what is my status?");
    }

    #[test]
    fn normalization_preserves_content_it_does_not_understand() {
        let normalized = normalize("réserve 2h — CS3télé <tag>").expect("accepted");
        assert_eq!(normalized.text, "réserve 2h — CS3télé <tag>");
    }

    #[test]
    fn empty_and_whitespace_only_input_is_rejected() {
        for input in ["", "   ", "\n\t  \r\n"] {
            let error = normalize(input).expect_err("rejected");
            assert_eq!(error.code(), "invalid_input");
        }
    }
}
