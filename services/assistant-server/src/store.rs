//! Postgres implementation of [`ActionStore`].
//!
//! Lives here, not in `assistant-core`, because `sqlx` is infrastructure and the
//! core depends on interfaces (ADR-0003 applied to storage).
//!
//! The two transactions that matter are documented at their call sites:
//! [`PostgresActionStore::claim_approval`], which must produce exactly one
//! winner under concurrency, and
//! [`PostgresActionStore::complete_execution`], which must not let an execution
//! finish without an audit row.

use assistant_core::actions::{
    ActionStore, ApprovalDecision, ApprovalId, ApprovalRequest, ApprovalTransitions, AuditEvent,
    ExecutionId, ExecutionStatus, RecordedDecision, StoreError, ToolExecution,
};
use assistant_tools::{ApprovalStatus, RiskLevel};
use async_trait::async_trait;
use sqlx::{PgPool, Row, postgres::PgRow};
use time::OffsetDateTime;
use uuid::Uuid;

pub struct PostgresActionStore {
    pool: PgPool,
}

impl PostgresActionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn backend(error: sqlx::Error) -> StoreError {
    StoreError::Backend(Box::new(error))
}

fn risk_to_str(risk: RiskLevel) -> &'static str {
    match risk {
        RiskLevel::Green => "green",
        RiskLevel::Yellow => "yellow",
        RiskLevel::Orange => "orange",
        RiskLevel::Red => "red",
    }
}

fn risk_from_str(value: &str) -> Result<RiskLevel, StoreError> {
    Ok(match value {
        "green" => RiskLevel::Green,
        "yellow" => RiskLevel::Yellow,
        "orange" => RiskLevel::Orange,
        "red" => RiskLevel::Red,
        other => {
            return Err(StoreError::Backend(
                format!("unknown risk level in database: {other}").into(),
            ));
        }
    })
}

fn decision_from_str(value: &str) -> Result<RecordedDecision, StoreError> {
    Ok(match value {
        "allow" => RecordedDecision::Allow,
        "require_approval" => RecordedDecision::RequireApproval,
        "deny" => RecordedDecision::Deny,
        other => {
            return Err(StoreError::Backend(
                format!("unknown decision in database: {other}").into(),
            ));
        }
    })
}

fn execution_status_from_str(value: &str) -> Result<ExecutionStatus, StoreError> {
    Ok(match value {
        "proposed" => ExecutionStatus::Proposed,
        "awaiting_approval" => ExecutionStatus::AwaitingApproval,
        "running" => ExecutionStatus::Running,
        "succeeded" => ExecutionStatus::Succeeded,
        "failed" => ExecutionStatus::Failed,
        "cancelled" => ExecutionStatus::Cancelled,
        other => {
            return Err(StoreError::Backend(
                format!("unknown execution status in database: {other}").into(),
            ));
        }
    })
}

fn approval_status_from_str(value: &str) -> Result<ApprovalStatus, StoreError> {
    Ok(match value {
        "requested" => ApprovalStatus::Requested,
        "approved" => ApprovalStatus::Approved,
        "rejected" => ApprovalStatus::Rejected,
        "expired" => ApprovalStatus::Expired,
        "cancelled" => ApprovalStatus::Cancelled,
        other => {
            return Err(StoreError::Backend(
                format!("unknown approval status in database: {other}").into(),
            ));
        }
    })
}

fn execution_from_row(row: &PgRow) -> Result<ToolExecution, StoreError> {
    Ok(ToolExecution {
        id: row.try_get("id").map_err(backend)?,
        turn_id: row.try_get("turn_id").map_err(backend)?,
        conversation_id: row.try_get("conversation_id").map_err(backend)?,
        principal_id: row.try_get("principal_id").map_err(backend)?,
        tool_name: row.try_get("tool_name").map_err(backend)?,
        arguments: row.try_get("arguments").map_err(backend)?,
        risk: risk_from_str(row.try_get("risk").map_err(backend)?)?,
        decision: decision_from_str(row.try_get("decision").map_err(backend)?)?,
        status: execution_status_from_str(row.try_get("status").map_err(backend)?)?,
        approval_id: None,
        created_at: row.try_get("created_at").map_err(backend)?,
        started_at: row.try_get("started_at").map_err(backend)?,
        completed_at: row.try_get("completed_at").map_err(backend)?,
        outcome: row.try_get("outcome").map_err(backend)?,
    })
}

fn approval_from_row(row: &PgRow) -> Result<ApprovalRequest, StoreError> {
    Ok(ApprovalRequest {
        id: row.try_get("id").map_err(backend)?,
        execution_id: row.try_get("execution_id").map_err(backend)?,
        principal_id: row.try_get("principal_id").map_err(backend)?,
        tool_name: row.try_get("tool_name").map_err(backend)?,
        risk: risk_from_str(row.try_get("risk").map_err(backend)?)?,
        reason: row.try_get("reason").map_err(backend)?,
        status: approval_status_from_str(row.try_get("status").map_err(backend)?)?,
        created_at: row.try_get("created_at").map_err(backend)?,
        expires_at: row.try_get("expires_at").map_err(backend)?,
        resolved_at: row.try_get("resolved_at").map_err(backend)?,
        resolved_by: row.try_get("resolved_by").map_err(backend)?,
    })
}

#[async_trait]
impl ActionStore for PostgresActionStore {
    /// One transaction: an approval must never reference an execution that was
    /// not written.
    async fn record_proposal(
        &self,
        execution: &ToolExecution,
        approval: Option<&ApprovalRequest>,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        sqlx::query(
            r#"
            insert into tool_executions
                (id, turn_id, conversation_id, principal_id, tool_name, arguments,
                 risk, decision, status, created_at, started_at, completed_at, outcome)
            values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
        )
        .bind(execution.id)
        .bind(execution.turn_id)
        .bind(execution.conversation_id)
        .bind(execution.principal_id)
        .bind(&execution.tool_name)
        .bind(&execution.arguments)
        .bind(risk_to_str(execution.risk))
        .bind(execution.decision.as_str())
        .bind(execution.status.as_str())
        .bind(execution.created_at)
        .bind(execution.started_at)
        .bind(execution.completed_at)
        .bind(&execution.outcome)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;

        if let Some(approval) = approval {
            sqlx::query(
                r#"
                insert into approval_requests
                    (id, execution_id, principal_id, tool_name, risk, reason,
                     status, created_at, expires_at, resolved_at, resolved_by)
                values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                "#,
            )
            .bind(approval.id)
            .bind(approval.execution_id)
            .bind(approval.principal_id)
            .bind(&approval.tool_name)
            .bind(risk_to_str(approval.risk))
            .bind(&approval.reason)
            .bind(ApprovalTransitions::as_str(approval.status))
            .bind(approval.created_at)
            .bind(approval.expires_at)
            .bind(approval.resolved_at)
            .bind(approval.resolved_by)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        }

        tx.commit().await.map_err(backend)
    }

    /// The concurrency-critical path.
    ///
    /// `SELECT ... FOR UPDATE` takes a row lock for the life of the transaction,
    /// so a second concurrent call blocks until the first commits and then sees
    /// the already-resolved status. That is what makes a double-tapped Approve
    /// button send one email rather than two.
    ///
    /// Ownership is part of the `WHERE` clause rather than a check afterwards,
    /// and a mismatch reports `ApprovalNotFound` — indistinguishable from an id
    /// that does not exist, so a caller cannot probe for other users' approvals.
    async fn claim_approval(
        &self,
        id: ApprovalId,
        resolver: Uuid,
        outcome: ApprovalStatus,
    ) -> Result<ApprovalDecision, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        let row = sqlx::query(
            r#"
            select * from approval_requests
            where id = $1 and principal_id = $2
            for update
            "#,
        )
        .bind(id)
        .bind(resolver)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;

        let Some(row) = row else {
            return Err(StoreError::ApprovalNotFound(id));
        };
        let approval = approval_from_row(&row)?;

        if ApprovalTransitions::is_terminal(approval.status) {
            tx.rollback().await.map_err(backend)?;
            return Ok(ApprovalDecision::AlreadyResolved {
                status: approval.status,
            });
        }

        let now = OffsetDateTime::now_utc();

        // Expiry is enforced here, on the write path, so an approval cannot be
        // executed late merely because no cleanup job ran.
        if approval.has_expired(now) {
            sqlx::query(
                "update approval_requests set status = 'expired', resolved_at = $2 where id = $1",
            )
            .bind(id)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

            sqlx::query(
                "update tool_executions set status = 'cancelled', completed_at = $2 where id = $1",
            )
            .bind(approval.execution_id)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

            tx.commit().await.map_err(backend)?;
            return Ok(ApprovalDecision::Expired);
        }

        if !ApprovalTransitions::can_transition_to(approval.status, outcome) {
            tx.rollback().await.map_err(backend)?;
            return Err(StoreError::InvalidTransition {
                kind: "approval",
                from: ApprovalTransitions::as_str(approval.status).to_string(),
                to: ApprovalTransitions::as_str(outcome).to_string(),
            });
        }

        sqlx::query(
            "update approval_requests set status = $2, resolved_at = $3, resolved_by = $4 where id = $1",
        )
        .bind(id)
        .bind(ApprovalTransitions::as_str(outcome))
        .bind(now)
        .bind(resolver)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;

        // Approved moves the execution to Running in the same transaction, so
        // "approved but not runnable" is not a state the database can hold.
        // Anything else cancels it.
        let (next_status, started_at, completed_at) = match outcome {
            ApprovalStatus::Approved => (ExecutionStatus::Running, Some(now), None),
            _ => (ExecutionStatus::Cancelled, None, Some(now)),
        };

        let execution_row = sqlx::query(
            r#"
            update tool_executions
            set status = $2, started_at = coalesce($3, started_at), completed_at = $4
            where id = $1 and status = 'awaiting_approval'
            returning *
            "#,
        )
        .bind(approval.execution_id)
        .bind(next_status.as_str())
        .bind(started_at)
        .bind(completed_at)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;

        let Some(execution_row) = execution_row else {
            // The execution was not awaiting approval. Refuse rather than force
            // it: this is the `Cancelled -> Running` case the state machine
            // exists to prevent.
            tx.rollback().await.map_err(backend)?;
            return Err(StoreError::InvalidTransition {
                kind: "execution",
                from: "not awaiting_approval".to_string(),
                to: next_status.as_str().to_string(),
            });
        };

        let mut execution = execution_from_row(&execution_row)?;
        execution.approval_id = Some(approval.id);

        let mut approval = approval;
        approval.status = outcome;
        approval.resolved_at = Some(now);
        approval.resolved_by = Some(resolver);

        tx.commit().await.map_err(backend)?;

        Ok(ApprovalDecision::Claimed {
            approval: Box::new(approval),
            execution: Box::new(execution),
        })
    }

    async fn set_execution_status(
        &self,
        id: ExecutionId,
        next: ExecutionStatus,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        let row = sqlx::query("select status from tool_executions where id = $1 for update")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(backend)?;

        let Some(row) = row else {
            return Err(StoreError::ExecutionNotFound(id));
        };
        let current = execution_status_from_str(row.try_get("status").map_err(backend)?)?;

        if !current.can_transition_to(next) {
            tx.rollback().await.map_err(backend)?;
            return Err(StoreError::InvalidTransition {
                kind: "execution",
                from: current.as_str().to_string(),
                to: next.as_str().to_string(),
            });
        }

        sqlx::query("update tool_executions set status = $2 where id = $1")
            .bind(id)
            .bind(next.as_str())
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

        tx.commit().await.map_err(backend)
    }

    /// One transaction, so a finished execution always has its audit row. If the
    /// audit insert fails the outcome is rolled back too, which is the safer of
    /// the two failure modes: retrying is recoverable, a silently unaudited
    /// consequential action is not.
    async fn complete_execution(
        &self,
        id: ExecutionId,
        status: ExecutionStatus,
        outcome: Option<serde_json::Value>,
        audit: &AuditEvent,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        let updated = sqlx::query(
            r#"
            update tool_executions
            set status = $2, completed_at = $3, outcome = $4
            where id = $1
            returning id
            "#,
        )
        .bind(id)
        .bind(status.as_str())
        .bind(OffsetDateTime::now_utc())
        .bind(&outcome)
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;

        if updated.is_none() {
            return Err(StoreError::ExecutionNotFound(id));
        }

        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(backend)
    }

    async fn record_audit(&self, audit: &AuditEvent) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(backend)
    }

    async fn pending_approvals(
        &self,
        principal_id: Uuid,
    ) -> Result<Vec<ApprovalRequest>, StoreError> {
        let rows = sqlx::query(
            r#"
            select * from approval_requests
            where principal_id = $1 and status = 'requested' and expires_at > now()
            order by created_at desc
            "#,
        )
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        rows.iter().map(approval_from_row).collect()
    }

    async fn approval(&self, id: ApprovalId) -> Result<ApprovalRequest, StoreError> {
        let row = sqlx::query("select * from approval_requests where id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?
            .ok_or(StoreError::ApprovalNotFound(id))?;
        approval_from_row(&row)
    }

    async fn execution(&self, id: ExecutionId) -> Result<ToolExecution, StoreError> {
        let row = sqlx::query("select * from tool_executions where id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?
            .ok_or(StoreError::ExecutionNotFound(id))?;
        execution_from_row(&row)
    }

    async fn audit_for_turn(&self, turn_id: Uuid) -> Result<Vec<AuditEvent>, StoreError> {
        let rows = sqlx::query("select * from audit_events where turn_id = $1 order by at asc")
            .bind(turn_id)
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;

        rows.iter()
            .map(|row| {
                let approval_outcome: Option<String> =
                    row.try_get("approval_outcome").map_err(backend)?;

                Ok(AuditEvent {
                    id: row.try_get("id").map_err(backend)?,
                    at: row.try_get("at").map_err(backend)?,
                    principal_id: row.try_get("principal_id").map_err(backend)?,
                    turn_id: row.try_get("turn_id").map_err(backend)?,
                    execution_id: row.try_get("execution_id").map_err(backend)?,
                    tool_name: row.try_get("tool_name").map_err(backend)?,
                    risk: risk_from_str(row.try_get("risk").map_err(backend)?)?,
                    decision: decision_from_str(row.try_get("decision").map_err(backend)?)?,
                    approval_id: row.try_get("approval_id").map_err(backend)?,
                    approval_outcome: approval_outcome
                        .as_deref()
                        .map(approval_status_from_str)
                        .transpose()?,
                    execution_status: execution_status_from_str(
                        row.try_get("execution_status").map_err(backend)?,
                    )?,
                    summary: row.try_get("summary").map_err(backend)?,
                })
            })
            .collect()
    }
}

async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    audit: &AuditEvent,
) -> Result<(), StoreError> {
    sqlx::query(
        r#"
        insert into audit_events
            (id, at, principal_id, turn_id, execution_id, tool_name, risk,
             decision, approval_id, approval_outcome, execution_status, summary)
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
    )
    .bind(audit.id)
    .bind(audit.at)
    .bind(audit.principal_id)
    .bind(audit.turn_id)
    .bind(audit.execution_id)
    .bind(&audit.tool_name)
    .bind(risk_to_str(audit.risk))
    .bind(audit.decision.as_str())
    .bind(audit.approval_id)
    .bind(audit.approval_outcome.map(ApprovalTransitions::as_str))
    .bind(audit.execution_status.as_str())
    .bind(&audit.summary)
    .execute(&mut **tx)
    .await
    .map_err(backend)?;

    Ok(())
}
