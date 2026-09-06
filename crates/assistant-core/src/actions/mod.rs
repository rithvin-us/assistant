//! Durable actions: approvals, tool executions and audit.
//!
//! Milestone 2 treated an approval as a dead end — the turn stopped and the
//! proposed action was lost. This module makes the action durable instead, so a
//! user can approve it minutes later, from a different device, after the server
//! has restarted.
//!
//! Three records, one lifecycle:
//!
//! ```text
//! ToolCall proposed
//!    │
//!    ├─ ToolExecution   (what the assistant wants to do, and how far it got)
//!    ├─ ApprovalRequest (the question put to the user, if policy asked one)
//!    └─ AuditEvent*     (an append-only record of what actually happened)
//! ```
//!
//! The security property this module exists to preserve: **an approval
//! authorises one specific persisted action, not a tool in general**. There is no
//! representation here of "the user trusts `gmail.send`" — only "the user
//! approved execution `<uuid>`, which was validated against the registry at the
//! moment it was proposed and is re-validated before it runs".

pub mod coordinator;
pub mod store;

use assistant_tools::{ApprovalStatus, RiskLevel};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

pub use coordinator::{ApprovalCoordinator, ResolutionOutcome};
pub use store::{ActionStore, ApprovalDecision, StoreError};

pub type ApprovalId = Uuid;
pub type ExecutionId = Uuid;
pub type AuditEventId = Uuid;

/// How long a pending approval stays actionable.
///
/// Domain policy, not a magic number scattered through call sites. An approval
/// that has been sitting unanswered for this long is far more likely to be a
/// stale notification than a decision the user still wants to make, and leaving
/// a `Red` action executable indefinitely is a standing hazard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalPolicy {
    pub ttl: Duration,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self {
            ttl: Duration::minutes(15),
        }
    }
}

impl ApprovalPolicy {
    pub fn expires_at(&self, from: OffsetDateTime) -> OffsetDateTime {
        from + self.ttl
    }
}

/// Where a proposed action has got to.
///
/// Expiry is represented as `Cancelled` on the execution and `Expired` on the
/// approval: the execution was cancelled, and the approval record says why.
/// Duplicating "expired" on both would make the two able to disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    /// Validated and resolved against the registry, not yet authorised.
    Proposed,
    /// Policy demanded a human decision. Nothing has run.
    AwaitingApproval,
    /// Authorised and handed to the executor.
    Running,
    Succeeded,
    Failed,
    /// Denied, rejected, expired, or cancelled with the turn.
    Cancelled,
}

impl ExecutionStatus {
    /// Whether no further transition is possible.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// The transition table.
    ///
    /// Encoded once, here, rather than implied by the order of `if` statements
    /// across the codebase. The entry that matters most is the one that is
    /// absent: nothing reaches `Running` except from `Proposed` or
    /// `AwaitingApproval`, so `Cancelled -> Running` — a rejected action being
    /// executed anyway — is not representable.
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Proposed, Self::AwaitingApproval)
                | (Self::Proposed, Self::Running)
                | (Self::Proposed, Self::Cancelled)
                | (Self::AwaitingApproval, Self::Running)
                | (Self::AwaitingApproval, Self::Cancelled)
                | (Self::Running, Self::Succeeded)
                | (Self::Running, Self::Failed)
                | (Self::Running, Self::Cancelled)
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Transition rules for [`ApprovalStatus`], which lives in `assistant-tools`
/// alongside the rest of the tool vocabulary.
///
/// Takes `self` by value throughout: `ApprovalStatus` is a `Copy` fieldless
/// enum, so passing it by reference would cost a pointer to save nothing and
/// read worse at every call site. That is what the allow below is for.
#[allow(clippy::wrong_self_convention)]
pub trait ApprovalTransitions {
    fn is_terminal(self) -> bool;
    fn can_transition_to(self, next: Self) -> bool;
    fn as_str(self) -> &'static str;
}

impl ApprovalTransitions for ApprovalStatus {
    /// Every state except `Requested` is final. An approval is answered once.
    fn is_terminal(self) -> bool {
        !matches!(self, Self::Requested)
    }

    fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Requested, Self::Approved)
                | (Self::Requested, Self::Rejected)
                | (Self::Requested, Self::Expired)
                | (Self::Requested, Self::Cancelled)
        )
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("cannot move {kind} from {from} to {to}")]
pub struct InvalidTransition {
    pub kind: &'static str,
    pub from: &'static str,
    pub to: &'static str,
}

/// A proposed or completed tool execution.
///
/// `arguments` are the ones validated against the registry's `ToolSpec` at
/// proposal time. Persisting them is what makes resume possible: on approval the
/// server runs *these* arguments, not something the client sends later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolExecution {
    pub id: ExecutionId,
    /// Correlates every record belonging to one assistant turn.
    pub turn_id: Uuid,
    pub conversation_id: Uuid,
    /// The authenticated user this action belongs to. Never client-supplied.
    pub principal_id: Uuid,
    pub tool_name: String,
    /// Validated arguments. See ADR-0016 on what may be stored here.
    pub arguments: serde_json::Value,
    /// Authoritative risk, from the registry at proposal time.
    pub risk: RiskLevel,
    /// What deterministic policy decided, recorded as it was decided.
    pub decision: RecordedDecision,
    pub status: ExecutionStatus,
    pub approval_id: Option<ApprovalId>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    /// Tool output on success, or the tool's error message on failure.
    pub outcome: Option<serde_json::Value>,
}

/// A permission decision, flattened for storage.
///
/// `PermissionDecision` carries a reason string; this keeps the discriminant
/// separately so the column is queryable ("show me everything that was denied")
/// without parsing JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedDecision {
    Allow,
    RequireApproval,
    Deny,
}

impl RecordedDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireApproval => "require_approval",
            Self::Deny => "deny",
        }
    }
}

/// A question put to the user about one specific execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    /// The action this authorises. One approval, one execution.
    pub execution_id: ExecutionId,
    pub principal_id: Uuid,
    pub tool_name: String,
    pub risk: RiskLevel,
    /// Why policy asked. Written for a human to read in the approval sheet.
    pub reason: String,
    pub status: ApprovalStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub resolved_at: Option<OffsetDateTime>,
    /// Who answered. Always the authenticated principal, never a client-supplied
    /// identifier.
    pub resolved_by: Option<Uuid>,
}

impl ApprovalRequest {
    /// Whether this approval may still be acted on at `now`.
    ///
    /// Expiry is evaluated against the stored `expires_at` rather than a
    /// background sweeper, so an approval cannot become executable again just
    /// because a cleanup job failed to run.
    pub fn is_actionable(&self, now: OffsetDateTime) -> bool {
        self.status == ApprovalStatus::Requested && now < self.expires_at
    }

    pub fn has_expired(&self, now: OffsetDateTime) -> bool {
        self.status == ApprovalStatus::Requested && now >= self.expires_at
    }
}

/// An append-only record of something consequential.
///
/// Audit is not conversation history, not memory, and not a prompt dump. It
/// answers: who attempted what, under which turn, what policy decided, whether a
/// human was asked, what they said, and how it ended.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: AuditEventId,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub principal_id: Uuid,
    pub turn_id: Uuid,
    pub execution_id: ExecutionId,
    pub tool_name: String,
    pub risk: RiskLevel,
    pub decision: RecordedDecision,
    pub approval_id: Option<ApprovalId>,
    pub approval_outcome: Option<ApprovalStatus>,
    pub execution_status: ExecutionStatus,
    /// A short, sanitised description of the action.
    ///
    /// Deliberately not the arguments. See [`summarize`] and ADR-0016.
    pub summary: String,
}

/// Builds the human-readable summary stored on an audit event.
///
/// Names the tool and the *shape* of its arguments — the keys — never the
/// values. A calendar title is unremarkable; an argument called `body` on a mail
/// tool is the contents of someone's email. Since the audit trail cannot know
/// which is which, it records neither.
pub fn summarize(tool_name: &str, arguments: &serde_json::Value) -> String {
    match arguments.as_object() {
        Some(fields) if !fields.is_empty() => {
            let mut keys: Vec<&str> = fields.keys().map(String::as_str).collect();
            keys.sort_unstable();
            format!("{tool_name} with fields: {}", keys.join(", "))
        }
        _ => format!("{tool_name} with no arguments"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancelled_execution_can_never_start_running() {
        assert!(!ExecutionStatus::Cancelled.can_transition_to(ExecutionStatus::Running));
        assert!(!ExecutionStatus::Succeeded.can_transition_to(ExecutionStatus::Running));
        assert!(!ExecutionStatus::Failed.can_transition_to(ExecutionStatus::Running));
    }

    #[test]
    fn running_is_only_reachable_from_proposed_or_awaiting_approval() {
        let reachable: Vec<ExecutionStatus> = [
            ExecutionStatus::Proposed,
            ExecutionStatus::AwaitingApproval,
            ExecutionStatus::Running,
            ExecutionStatus::Succeeded,
            ExecutionStatus::Failed,
            ExecutionStatus::Cancelled,
        ]
        .into_iter()
        .filter(|s| s.can_transition_to(ExecutionStatus::Running))
        .collect();

        assert_eq!(
            reachable,
            vec![ExecutionStatus::Proposed, ExecutionStatus::AwaitingApproval]
        );
    }

    #[test]
    fn terminal_execution_states_permit_no_transition() {
        for terminal in [
            ExecutionStatus::Succeeded,
            ExecutionStatus::Failed,
            ExecutionStatus::Cancelled,
        ] {
            assert!(terminal.is_terminal());
            for next in [
                ExecutionStatus::Proposed,
                ExecutionStatus::AwaitingApproval,
                ExecutionStatus::Running,
                ExecutionStatus::Succeeded,
                ExecutionStatus::Failed,
                ExecutionStatus::Cancelled,
            ] {
                assert!(
                    !terminal.can_transition_to(next),
                    "{terminal:?} should not reach {next:?}"
                );
            }
        }
    }

    #[test]
    fn an_answered_approval_cannot_be_answered_again() {
        for answered in [
            ApprovalStatus::Approved,
            ApprovalStatus::Rejected,
            ApprovalStatus::Expired,
            ApprovalStatus::Cancelled,
        ] {
            assert!(ApprovalTransitions::is_terminal(answered));
            assert!(
                !ApprovalTransitions::can_transition_to(answered, ApprovalStatus::Approved),
                "{answered:?} was re-approved"
            );
        }
    }

    #[test]
    fn only_a_requested_approval_is_actionable_and_only_before_it_expires() {
        let now = OffsetDateTime::now_utc();
        let policy = ApprovalPolicy::default();

        let mut approval = ApprovalRequest {
            id: Uuid::new_v4(),
            execution_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            tool_name: "gmail.send".into(),
            risk: RiskLevel::Red,
            reason: "sends mail on your behalf".into(),
            status: ApprovalStatus::Requested,
            created_at: now,
            expires_at: policy.expires_at(now),
            resolved_at: None,
            resolved_by: None,
        };

        assert!(approval.is_actionable(now));
        assert!(!approval.has_expired(now));

        // One second past the deadline.
        let later = approval.expires_at + Duration::seconds(1);
        assert!(
            !approval.is_actionable(later),
            "an expired approval stayed actionable"
        );
        assert!(approval.has_expired(later));

        // Already answered: not actionable even well within the window.
        approval.status = ApprovalStatus::Rejected;
        assert!(!approval.is_actionable(now));
        assert!(
            !approval.has_expired(later),
            "a rejected approval must not become expired"
        );
    }

    #[test]
    fn the_audit_summary_names_argument_keys_but_never_their_values() {
        let summary = summarize(
            "gmail.send",
            &serde_json::json!({
                "to": "professor@example.edu",
                "body": "Dear Professor, my password is hunter2",
                "subject": "Late submission"
            }),
        );

        assert!(summary.contains("gmail.send"));
        assert!(summary.contains("body"));
        assert!(
            !summary.contains("hunter2"),
            "argument value leaked: {summary}"
        );
        assert!(
            !summary.contains("professor@example.edu"),
            "argument value leaked: {summary}"
        );
        assert!(
            !summary.contains("Dear Professor"),
            "argument value leaked: {summary}"
        );
    }

    #[test]
    fn summarizing_non_object_arguments_does_not_panic_or_leak() {
        assert_eq!(
            summarize("notes.read", &serde_json::json!({})),
            "notes.read with no arguments"
        );
        assert_eq!(
            summarize("notes.read", &serde_json::json!("secret-value")),
            "notes.read with no arguments"
        );
    }
}
