//! Proposing, resuming and auditing durable actions.
//!
//! One type owns the whole lifecycle so there is exactly one place where a tool
//! can start running. `assistant-server` calls [`ApprovalCoordinator::resolve`]
//! when a user answers; the orchestrator calls [`ApprovalCoordinator::propose`]
//! when policy asks a question. Neither of them executes anything itself.
//!
//! The rule this file exists to enforce: **approval is permission to run one
//! specific persisted action, and it is re-checked against live policy before
//! the tool is touched.** An approval granted ten minutes ago does not survive
//! the tool being removed from the registry, the user losing a scope, or the
//! tool being added to a blocklist in between.

use std::sync::Arc;

use assistant_auth::Principal;
use assistant_tools::{ApprovalStatus, PermissionDecision, ToolCall};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    CoreError,
    actions::{
        ActionStore, ApprovalDecision, ApprovalId, ApprovalPolicy, ApprovalRequest, AuditEvent,
        ExecutionStatus, RecordedDecision, StoreError, ToolExecution, summarize,
    },
    event::{DomainEvent, EventBus},
    executor::ToolExecutor,
};

/// The outcome of answering an approval, as the transport should report it.
#[derive(Debug, Clone)]
pub enum ResolutionOutcome {
    /// The tool ran. `ok` distinguishes a successful run from a handled failure.
    Executed { execution_id: Uuid, ok: bool },
    /// The user said no. Nothing ran.
    Rejected { execution_id: Uuid },
    /// The window had closed. Nothing ran.
    Expired { approval_id: ApprovalId },
    /// Someone had already answered. Nothing ran, and nothing ran a second time.
    AlreadyResolved { status: ApprovalStatus },
    /// Policy changed while the approval was pending. Nothing ran.
    NoLongerPermitted { execution_id: Uuid, reason: String },
}

pub struct ApprovalCoordinator {
    store: Arc<dyn ActionStore>,
    executor: Arc<ToolExecutor>,
    policy: ApprovalPolicy,
    events: Option<EventBus>,
}

impl ApprovalCoordinator {
    pub fn new(
        store: Arc<dyn ActionStore>,
        executor: Arc<ToolExecutor>,
        policy: ApprovalPolicy,
        events: Option<EventBus>,
    ) -> Self {
        Self {
            store,
            executor,
            policy,
            events,
        }
    }

    pub fn store(&self) -> &Arc<dyn ActionStore> {
        &self.store
    }

    /// Persists an action that policy has held for approval.
    ///
    /// The arguments written here are the ones already validated against the
    /// registry's `ToolSpec`. Resume replays *these*, never anything a client
    /// sends later — which is why the mobile app can only ever say "approve
    /// `<uuid>`" and never "approve, and by the way here are the arguments".
    #[tracing::instrument(skip_all, fields(tool = %call.name, turn_id = %turn_id))]
    pub async fn propose(
        &self,
        call: &ToolCall,
        principal: &Principal,
        turn_id: Uuid,
        conversation_id: Uuid,
        risk: assistant_tools::RiskLevel,
        reason: &str,
    ) -> Result<ApprovalRequest, CoreError> {
        let now = OffsetDateTime::now_utc();

        let execution = ToolExecution {
            id: Uuid::new_v4(),
            turn_id,
            conversation_id,
            principal_id: principal.user_id,
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            risk,
            decision: RecordedDecision::RequireApproval,
            status: ExecutionStatus::AwaitingApproval,
            approval_id: None,
            created_at: now,
            started_at: None,
            completed_at: None,
            outcome: None,
        };

        let approval = ApprovalRequest {
            id: Uuid::new_v4(),
            execution_id: execution.id,
            principal_id: principal.user_id,
            tool_name: call.name.clone(),
            risk,
            reason: reason.to_string(),
            status: ApprovalStatus::Requested,
            created_at: now,
            expires_at: self.policy.expires_at(now),
            resolved_at: None,
            resolved_by: None,
        };

        self.store
            .record_proposal(&execution, Some(&approval))
            .await
            .map_err(store_error)?;

        tracing::info!(
            approval_id = %approval.id,
            execution_id = %execution.id,
            risk = ?risk,
            "action held for approval"
        );

        self.publish(DomainEvent::ApprovalRequested {
            name: call.name.clone(),
            risk,
        });

        Ok(approval)
    }

    /// Answers an approval and, if approved and still permitted, runs it.
    ///
    /// `principal` is the authenticated caller. It is passed to the store, which
    /// scopes its lookup by owner, so a user cannot answer someone else's
    /// approval and cannot learn whether one exists.
    #[tracing::instrument(
        skip_all,
        fields(approval_id = %approval_id, outcome = ?outcome, execution_id = tracing::field::Empty)
    )]
    pub async fn resolve(
        &self,
        approval_id: ApprovalId,
        principal: &Principal,
        outcome: ApprovalStatus,
        cancel: &CancellationToken,
    ) -> Result<ResolutionOutcome, CoreError> {
        // Atomic. Two concurrent calls produce one winner; the loser gets
        // AlreadyResolved and no means to execute.
        let claimed = self
            .store
            .claim_approval(approval_id, principal.user_id, outcome)
            .await
            .map_err(store_error)?;

        let (approval, execution) = match claimed {
            ApprovalDecision::Claimed {
                approval,
                execution,
            } => (*approval, *execution),
            ApprovalDecision::AlreadyResolved { status } => {
                tracing::info!(?status, "duplicate approval resolution ignored");
                return Ok(ResolutionOutcome::AlreadyResolved { status });
            }
            ApprovalDecision::Expired => {
                tracing::info!("approval had expired");
                // The store already marked both records; the audit row is the
                // remaining obligation.
                let approval = self
                    .store
                    .approval(approval_id)
                    .await
                    .map_err(store_error)?;
                self.audit(&approval_expiry_audit(&approval)).await;
                return Ok(ResolutionOutcome::Expired { approval_id });
            }
        };

        tracing::Span::current().record("execution_id", tracing::field::display(execution.id));

        if outcome != ApprovalStatus::Approved {
            self.finish(
                &execution,
                ExecutionStatus::Cancelled,
                None,
                Some(&approval),
                outcome,
            )
            .await;
            return Ok(ResolutionOutcome::Rejected {
                execution_id: execution.id,
            });
        }

        // Re-validate. The approval said "yes to this action"; it did not freeze
        // the world. Between the question and the answer the tool may have been
        // unregistered, blocklisted, or the user may have lost a scope.
        let call = ToolCall {
            id: execution.id.to_string(),
            name: execution.tool_name.clone(),
            arguments: execution.arguments.clone(),
        };

        let spec = match self.executor.resolve(&call) {
            Ok(spec) => spec,
            Err(error) => {
                let reason = error.to_string();
                tracing::warn!(%reason, "approved tool is no longer registered");
                self.finish(
                    &execution,
                    ExecutionStatus::Cancelled,
                    None,
                    Some(&approval),
                    outcome,
                )
                .await;
                return Ok(ResolutionOutcome::NoLongerPermitted {
                    execution_id: execution.id,
                    reason,
                });
            }
        };

        // The authoritative risk is re-read from the registry, never from the
        // stored row and never from the client.
        match self.executor.decide(&spec, principal) {
            // Still requiring approval is the expected answer for a Red tool --
            // that is precisely the question the user just answered.
            PermissionDecision::Allow | PermissionDecision::RequireApproval { .. } => {}
            PermissionDecision::Deny { reason } => {
                tracing::warn!(%reason, "approved action is no longer permitted");
                self.finish(
                    &execution,
                    ExecutionStatus::Cancelled,
                    None,
                    Some(&approval),
                    outcome,
                )
                .await;
                return Ok(ResolutionOutcome::NoLongerPermitted {
                    execution_id: execution.id,
                    reason,
                });
            }
        }

        // The one execution path. Approval changed only whether we got here.
        let result = self
            .executor
            .run_authorized_with_user(&call, &spec, Some(execution.principal_id), cancel)
            .await;

        let (status, outcome_json, ok) = match result {
            Ok(tool_result) => match tool_result.result {
                Ok(value) => (ExecutionStatus::Succeeded, Some(value), true),
                Err(message) => (
                    ExecutionStatus::Failed,
                    Some(serde_json::json!({ "error": message })),
                    false,
                ),
            },
            Err(error) => (
                ExecutionStatus::Failed,
                Some(serde_json::json!({ "error": error.to_string() })),
                false,
            ),
        };

        self.finish(&execution, status, outcome_json, Some(&approval), outcome)
            .await;

        Ok(ResolutionOutcome::Executed {
            execution_id: execution.id,
            ok,
        })
    }

    /// Writes the terminal execution state and its audit event together.
    ///
    /// Failures are logged rather than propagated: the action has already
    /// happened, and turning a storage problem into a caller-visible error would
    /// invite a retry that ran it a second time.
    async fn finish(
        &self,
        execution: &ToolExecution,
        status: ExecutionStatus,
        outcome: Option<serde_json::Value>,
        approval: Option<&ApprovalRequest>,
        approval_outcome: ApprovalStatus,
    ) {
        let audit = AuditEvent {
            id: Uuid::new_v4(),
            at: OffsetDateTime::now_utc(),
            principal_id: execution.principal_id,
            turn_id: execution.turn_id,
            execution_id: execution.id,
            tool_name: execution.tool_name.clone(),
            risk: execution.risk,
            decision: execution.decision,
            approval_id: approval.map(|a| a.id),
            approval_outcome: Some(approval_outcome),
            execution_status: status,
            summary: summarize(&execution.tool_name, &execution.arguments),
        };

        if let Err(error) = self
            .store
            .complete_execution(execution.id, status, outcome, &audit)
            .await
        {
            tracing::error!(
                execution_id = %execution.id,
                %error,
                "could not record the outcome of a completed action"
            );
        }

        self.publish(DomainEvent::ToolCompleted {
            name: execution.tool_name.clone(),
            ok: status == ExecutionStatus::Succeeded,
        });
    }

    async fn audit(&self, event: &AuditEvent) {
        if let Err(error) = self.store.record_audit(event).await {
            tracing::error!(%error, "could not write audit event");
        }
    }

    fn publish(&self, event: DomainEvent) {
        if let Some(bus) = &self.events {
            bus.publish(event);
        }
    }
}

fn approval_expiry_audit(approval: &ApprovalRequest) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        at: OffsetDateTime::now_utc(),
        principal_id: approval.principal_id,
        // The originating turn is not carried on the approval row; the execution
        // it points at holds it, and the execution id is recorded here.
        turn_id: Uuid::nil(),
        execution_id: approval.execution_id,
        tool_name: approval.tool_name.clone(),
        risk: approval.risk,
        decision: RecordedDecision::RequireApproval,
        approval_id: Some(approval.id),
        approval_outcome: Some(ApprovalStatus::Expired),
        execution_status: ExecutionStatus::Cancelled,
        summary: format!("{} expired without an answer", approval.tool_name),
    }
}

fn store_error(error: StoreError) -> CoreError {
    match error {
        StoreError::ApprovalNotFound(id) => CoreError::ApprovalNotFound(id),
        other => CoreError::Internal(Box::new(other)),
    }
}
