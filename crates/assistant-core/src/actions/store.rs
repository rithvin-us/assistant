//! The durable-storage seam.
//!
//! `assistant-core` defines what must be stored and what atomicity it needs; it
//! does not know that Postgres exists. The SQL implementation lives in
//! `assistant-server`, which already owns `sqlx` and the connection pool — the
//! same rule that keeps model providers out of the core (ADR-0003) applied to
//! infrastructure.
//!
//! The trait is written so the *safety* properties are the store's contract, not
//! the caller's discipline:
//!
//! * [`ActionStore::claim_approval`] is the only way to answer an approval, and
//!   it is specified to be atomic. A caller cannot forget to lock, because there
//!   is no read-then-write pair to get wrong.
//! * Recording an execution's outcome writes the audit event in the same
//!   transaction, so a completed action cannot exist without an audit trail.

use assistant_tools::ApprovalStatus;
use async_trait::async_trait;
use uuid::Uuid;

use super::{ApprovalId, ApprovalRequest, AuditEvent, ExecutionId, ExecutionStatus, ToolExecution};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("no approval with id {0}")]
    ApprovalNotFound(ApprovalId),
    #[error("no execution with id {0}")]
    ExecutionNotFound(ExecutionId),
    #[error("{kind} cannot move from {from} to {to}")]
    InvalidTransition {
        kind: &'static str,
        from: String,
        to: String,
    },
    #[error("durable store unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("durable store failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// What happened when a caller tried to answer an approval.
///
/// `AlreadyResolved` is the double-tap case and is deliberately not an error:
/// the second tap of an Approve button is a duplicate request, not a fault. The
/// caller is told the approval was already settled and — crucially — is *not*
/// handed a token to execute with.
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    /// This call won the race. The caller now owns the right to run the
    /// execution exactly once.
    Claimed {
        approval: Box<ApprovalRequest>,
        execution: Box<ToolExecution>,
    },
    /// Someone (or the same someone, twice) already answered it.
    AlreadyResolved { status: ApprovalStatus },
    /// The window closed. The store has marked it expired.
    Expired,
}

#[async_trait]
pub trait ActionStore: Send + Sync {
    /// Persists a proposed execution and, when policy demanded one, its approval
    /// request. Both are written in a single transaction: an approval that
    /// referenced a missing execution would be unanswerable.
    async fn record_proposal(
        &self,
        execution: &ToolExecution,
        approval: Option<&ApprovalRequest>,
    ) -> Result<(), StoreError>;

    /// Atomically answers an approval and moves its execution forward.
    ///
    /// Implementations **must** make the read-check-write sequence atomic —
    /// `SELECT ... FOR UPDATE` inside a transaction, or an equivalent conditional
    /// update. Two concurrent approvals of the same id must produce exactly one
    /// [`ApprovalDecision::Claimed`]; the loser sees `AlreadyResolved`. This is
    /// the single guarantee that stops a double-tapped Approve button from
    /// sending an email twice.
    ///
    /// `resolver` is the authenticated principal. Implementations must reject an
    /// attempt by a principal that does not own the approval, reporting
    /// [`StoreError::ApprovalNotFound`] rather than a distinguishable
    /// "forbidden" — a caller must not be able to probe for the existence of
    /// other users' approvals.
    ///
    /// An approval past its deadline is marked expired here, so expiry is
    /// enforced on the read path and does not depend on a sweeper having run.
    async fn claim_approval(
        &self,
        id: ApprovalId,
        resolver: Uuid,
        outcome: ApprovalStatus,
    ) -> Result<ApprovalDecision, StoreError>;

    /// Moves an execution to a new status, rejecting invalid transitions.
    async fn set_execution_status(
        &self,
        id: ExecutionId,
        next: ExecutionStatus,
    ) -> Result<(), StoreError>;

    /// Records a terminal outcome and its audit event atomically.
    ///
    /// One transaction, so an action cannot succeed without leaving an audit
    /// trail — a completed execution with no corresponding audit row would be
    /// exactly the record someone would want to be missing.
    async fn complete_execution(
        &self,
        id: ExecutionId,
        status: ExecutionStatus,
        outcome: Option<serde_json::Value>,
        audit: &AuditEvent,
    ) -> Result<(), StoreError>;

    /// Appends an audit event on its own, for outcomes with no execution to
    /// complete — a denial, or an approval that expired unanswered.
    async fn record_audit(&self, audit: &AuditEvent) -> Result<(), StoreError>;

    /// Approvals still awaiting an answer for this principal.
    ///
    /// Scoped by principal in the query itself, not filtered afterwards, so
    /// there is no code path that can accidentally return someone else's.
    /// Approvals whose deadline has passed are excluded.
    async fn pending_approvals(
        &self,
        principal_id: Uuid,
    ) -> Result<Vec<ApprovalRequest>, StoreError>;

    async fn approval(&self, id: ApprovalId) -> Result<ApprovalRequest, StoreError>;

    async fn execution(&self, id: ExecutionId) -> Result<ToolExecution, StoreError>;

    /// Audit events for one turn, oldest first.
    async fn audit_for_turn(&self, turn_id: Uuid) -> Result<Vec<AuditEvent>, StoreError>;
}
