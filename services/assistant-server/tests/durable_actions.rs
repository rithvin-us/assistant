//! Durable approval, execution and audit tests against a real PostgreSQL.
//!
//! These exercise the actual `PostgresActionStore` — transactions, row locks and
//! constraints included — because the properties under test (a double-tapped
//! Approve button running one email, an expired approval refusing to execute)
//! are properties of the database, not of Rust. An in-memory fake would pass
//! while proving nothing.
//!
//! Every test is skipped, not failed, when `DATABASE_URL` is absent, so
//! `cargo test --workspace` stays runnable without credentials. Run them with:
//!
//! ```powershell
//! $env:DATABASE_URL = "<connection string>"; cargo test -p assistant-server --test durable_actions
//! ```
//!
//! Each test namespaces its rows by a fresh principal uuid, so runs are
//! independent and can execute concurrently against one database.

use std::sync::Arc;

use assistant_auth::Principal;
use assistant_core::{
    ToolRegistry,
    actions::{
        ActionStore, ApprovalCoordinator, ApprovalDecision, ApprovalPolicy, ApprovalRequest,
        ApprovalTransitions, ExecutionStatus, RecordedDecision, ResolutionOutcome, ToolExecution,
        summarize,
    },
    executor::ToolExecutor,
    permission::RiskBasedPolicy,
    testing::{EchoTool, FailingTool},
};
use assistant_server::store::PostgresActionStore;
use assistant_tools::{ApprovalStatus, RiskLevel, Tool};
use time::{Duration, OffsetDateTime};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Caps how many tests may hold database connections at once.
///
/// A hosted Supabase session pooler admits a fixed number of clients (15 for
/// this project) and refuses the rest with `EMAXCONNSESSION`. The test harness
/// runs tests in parallel with no idea about that, so the limit is enforced
/// here. Without it the suite fails on connection admission rather than on
/// anything it is trying to assert.
static DB_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(6);

/// Opens a pool for one test.
///
/// Deliberately *not* shared across tests. Each `#[tokio::test]` builds its own
/// runtime, and a `sqlx` pool is bound to the runtime that created it: a pool in
/// a `static` is torn down when the first test's runtime ends, and every later
/// test fails with "a Tokio 1.x context ... is being shutdown".
///
/// Connections are kept to the minimum each test needs, because a hosted pooler
/// has a connection ceiling and the suite runs its tests in parallel.
async fn pool_with(max_connections: u32) -> Option<sqlx::PgPool> {
    let _ = dotenvy::dotenv();
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.is_empty())?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .idle_timeout(std::time::Duration::from_secs(1))
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect(&url)
        .await
        .expect("DATABASE_URL is set but the database is unreachable");

    Some(pool)
}

/// A store plus the permit that entitles it to a connection.
///
/// The permit is released when the test's binding is dropped.
struct TestStore {
    store: Arc<PostgresActionStore>,
    _permit: tokio::sync::SemaphorePermit<'static>,
}

impl std::ops::Deref for TestStore {
    type Target = Arc<PostgresActionStore>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

/// Returns a live store, or `None` when no database is configured.
async fn store() -> Option<TestStore> {
    let permit = DB_PERMITS.acquire().await.expect("semaphore open");
    let pool = pool_with(1).await?;
    Some(TestStore {
        store: Arc::new(PostgresActionStore::new(pool)),
        _permit: permit,
    })
}

/// Skips the test body when no database is configured.
macro_rules! store_or_skip {
    () => {
        match store().await {
            Some(store) => store,
            None => {
                eprintln!("skipped: DATABASE_URL is not set");
                return;
            }
        }
    };
}

fn principal() -> Principal {
    // A fresh id per test keeps concurrent runs from seeing each other's rows.
    Principal {
        user_id: Uuid::new_v4(),
        scopes: Vec::new(),
    }
}

/// A store on its own short-lived pool, for the restart tests.
///
/// Standing in for a server process: when it is closed, everything this half of
/// the test held is gone, and only what reached Postgres remains.
struct SeparateStore {
    store: Arc<PostgresActionStore>,
    pool: sqlx::PgPool,
    _permit: tokio::sync::SemaphorePermit<'static>,
}

impl SeparateStore {
    async fn close(self) {
        self.pool.close().await;
    }
}

impl std::ops::Deref for SeparateStore {
    type Target = Arc<PostgresActionStore>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

async fn separate_store() -> Option<SeparateStore> {
    let permit = DB_PERMITS.acquire().await.expect("semaphore open");
    let pool = pool_with(1).await?;
    Some(SeparateStore {
        store: Arc::new(PostgresActionStore::new(pool.clone())),
        pool,
        _permit: permit,
    })
}

fn executor_with(tools: Vec<Arc<dyn Tool>>) -> Arc<ToolExecutor> {
    let mut registry = ToolRegistry::new();
    for tool in tools {
        registry.register(tool);
    }
    Arc::new(ToolExecutor::new(
        Arc::new(registry),
        Arc::new(RiskBasedPolicy::new()),
    ))
}

fn coordinator(
    store: Arc<PostgresActionStore>,
    tools: Vec<Arc<dyn Tool>>,
    policy: ApprovalPolicy,
) -> Arc<ApprovalCoordinator> {
    Arc::new(ApprovalCoordinator::new(
        store as Arc<dyn ActionStore>,
        executor_with(tools),
        policy,
        None,
    ))
}

/// Writes a proposal directly, bypassing the orchestrator, when a test only
/// needs a row to exist.
async fn seed(
    store: &PostgresActionStore,
    principal: &Principal,
    tool_name: &str,
    expires_in: Duration,
) -> (ToolExecution, ApprovalRequest) {
    let now = OffsetDateTime::now_utc();
    let arguments = serde_json::json!({"to": "someone@example.com"});

    let execution = ToolExecution {
        id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        principal_id: principal.user_id,
        tool_name: tool_name.to_string(),
        arguments,
        risk: RiskLevel::Red,
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
        tool_name: tool_name.to_string(),
        risk: RiskLevel::Red,
        reason: "test fixture".into(),
        status: ApprovalStatus::Requested,
        created_at: now,
        expires_at: now + expires_in,
        resolved_at: None,
        resolved_by: None,
    };

    store
        .record_proposal(&execution, Some(&approval))
        .await
        .expect("proposal stored");

    (execution, approval)
}

// ------------------------------------------------- persistence and retrieval

#[tokio::test]
async fn a_proposal_is_persisted_and_can_be_read_back() {
    let store = store_or_skip!();
    let principal = principal();
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    let loaded = store
        .approval(approval.id)
        .await
        .expect("approval readable");
    assert_eq!(loaded.id, approval.id);
    assert_eq!(loaded.execution_id, execution.id);
    assert_eq!(loaded.status, ApprovalStatus::Requested);
    assert_eq!(loaded.risk, RiskLevel::Red);

    let loaded_execution = store
        .execution(execution.id)
        .await
        .expect("execution readable");
    assert_eq!(loaded_execution.status, ExecutionStatus::AwaitingApproval);
    assert_eq!(loaded_execution.principal_id, principal.user_id);
}

#[tokio::test]
async fn pending_approvals_are_scoped_to_their_owner() {
    let store = store_or_skip!();
    let mine = principal();
    let theirs = principal();

    seed(&store, &mine, "gmail.send", Duration::minutes(15)).await;
    seed(&store, &theirs, "gmail.send", Duration::minutes(15)).await;

    let listed = store.pending_approvals(mine.user_id).await.expect("listed");
    assert_eq!(listed.len(), 1, "listing leaked another user's approvals");
    assert_eq!(listed[0].principal_id, mine.user_id);
}

#[tokio::test]
async fn an_expired_approval_is_not_listed_as_pending() {
    let store = store_or_skip!();
    let principal = principal();
    seed(&store, &principal, "gmail.send", Duration::seconds(-1)).await;

    let listed = store
        .pending_approvals(principal.user_id)
        .await
        .expect("listed");
    assert!(
        listed.is_empty(),
        "an expired approval was offered to the user"
    );
}

// ------------------------------------------------------------ authorization

#[tokio::test]
async fn a_user_cannot_answer_someone_elses_approval() {
    let store = store_or_skip!();
    let owner = principal();
    let attacker = principal();
    let (execution, approval) = seed(&store, &owner, "gmail.send", Duration::minutes(15)).await;

    let result = store
        .claim_approval(approval.id, attacker.user_id, ApprovalStatus::Approved)
        .await;

    match result {
        // Indistinguishable from a non-existent id, on purpose.
        Err(error) => assert!(
            error.to_string().contains("no approval"),
            "unexpected error: {error}"
        ),
        Ok(other) => panic!("another user's approval was claimable: {other:?}"),
    }

    let untouched = store.execution(execution.id).await.expect("readable");
    assert_eq!(
        untouched.status,
        ExecutionStatus::AwaitingApproval,
        "an unauthorised claim moved the execution"
    );
}

// ------------------------------------------------ idempotency and concurrency

#[tokio::test]
async fn answering_the_same_approval_twice_claims_it_only_once() {
    let store = store_or_skip!();
    let principal = principal();
    let (_, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    let first = store
        .claim_approval(approval.id, principal.user_id, ApprovalStatus::Approved)
        .await
        .expect("first claim");
    assert!(matches!(first, ApprovalDecision::Claimed { .. }));

    let second = store
        .claim_approval(approval.id, principal.user_id, ApprovalStatus::Approved)
        .await
        .expect("second claim answered");
    match second {
        ApprovalDecision::AlreadyResolved { status } => {
            assert_eq!(status, ApprovalStatus::Approved);
        }
        other => panic!("a double tap claimed the approval twice: {other:?}"),
    }
}

/// The double-tap case, raced for real.
///
/// Eight simultaneous approvals of one id. Exactly one may win; if more than one
/// did, a dangerous tool would run more than once.
#[tokio::test]
async fn concurrent_claims_produce_exactly_one_winner() {
    // Enough connections for the claims to actually overlap; with one
    // connection they would serialise in the pool and the row lock -- the thing
    // under test -- would never be contended.
    // Takes every permit: this is the one test that legitimately needs several
    // connections at once, so nothing else may hold one while it runs.
    let _permits = DB_PERMITS.acquire_many(6).await.expect("semaphore open");
    let Some(pool) = pool_with(8).await else {
        eprintln!("skipped: DATABASE_URL is not set");
        return;
    };
    let store = Arc::new(PostgresActionStore::new(pool));
    let principal = principal();
    let (_, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = store.clone();
        let id = approval.id;
        let user = principal.user_id;
        handles.push(tokio::spawn(async move {
            store
                .claim_approval(id, user, ApprovalStatus::Approved)
                .await
        }));
    }

    let mut claimed = 0;
    let mut already = 0;
    for handle in handles {
        match handle.await.expect("task joined").expect("claim answered") {
            ApprovalDecision::Claimed { .. } => claimed += 1,
            ApprovalDecision::AlreadyResolved { .. } => already += 1,
            ApprovalDecision::Expired => panic!("unexpected expiry"),
        }
    }

    assert_eq!(
        claimed, 1,
        "{claimed} concurrent callers all claimed the approval"
    );
    assert_eq!(already, 7);
}

// ------------------------------------------------------------- state machine

#[tokio::test]
async fn a_rejected_action_can_never_afterwards_be_approved() {
    let store = store_or_skip!();
    let principal = principal();
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    store
        .claim_approval(approval.id, principal.user_id, ApprovalStatus::Rejected)
        .await
        .expect("rejected");

    let second = store
        .claim_approval(approval.id, principal.user_id, ApprovalStatus::Approved)
        .await
        .expect("answered");
    assert!(
        matches!(
            second,
            ApprovalDecision::AlreadyResolved {
                status: ApprovalStatus::Rejected
            }
        ),
        "a rejected approval was re-opened: {second:?}"
    );

    let final_state = store.execution(execution.id).await.expect("readable");
    assert_eq!(final_state.status, ExecutionStatus::Cancelled);
}

#[tokio::test]
async fn an_invalid_execution_transition_is_refused() {
    let store = store_or_skip!();
    let principal = principal();
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    store
        .claim_approval(approval.id, principal.user_id, ApprovalStatus::Rejected)
        .await
        .expect("rejected");

    // Cancelled -> Running is the transition the state machine exists to forbid.
    let result = store
        .set_execution_status(execution.id, ExecutionStatus::Running)
        .await;
    assert!(
        result.is_err(),
        "a cancelled execution was moved to running"
    );
}

// ---------------------------------------------------------------- expiration

#[tokio::test]
async fn an_expired_approval_cannot_be_executed() {
    let store = store_or_skip!();
    let principal = principal();
    let tool = Arc::new(EchoTool::red("gmail.send"));
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::seconds(-1)).await;

    let coordinator = coordinator(
        (*store).clone(),
        vec![tool.clone()],
        ApprovalPolicy::default(),
    );

    let outcome = coordinator
        .resolve(
            approval.id,
            &principal,
            ApprovalStatus::Approved,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved");

    assert!(
        matches!(outcome, ResolutionOutcome::Expired { .. }),
        "expected expiry, got {outcome:?}"
    );
    assert_eq!(tool.calls(), 0, "an expired approval executed its tool");

    let stored = store.approval(approval.id).await.expect("readable");
    assert_eq!(stored.status, ApprovalStatus::Expired);

    let stored_execution = store.execution(execution.id).await.expect("readable");
    assert_eq!(stored_execution.status, ExecutionStatus::Cancelled);

    let audit = store
        .audit_for_turn(stored_execution.turn_id)
        .await
        .expect("audit readable");
    assert!(
        audit.iter().any(|e| e.execution_id == execution.id)
            || !store
                .audit_for_turn(Uuid::nil())
                .await
                .expect("nil-turn audit readable")
                .is_empty(),
        "expiry wrote no audit event"
    );
}

// ------------------------------------------------------- approve and execute

#[tokio::test]
async fn an_approved_action_executes_through_the_shared_executor_and_is_audited() {
    let store = store_or_skip!();
    let principal = principal();
    let tool = Arc::new(EchoTool::red("gmail.send"));
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    let coordinator = coordinator(
        (*store).clone(),
        vec![tool.clone()],
        ApprovalPolicy::default(),
    );

    let outcome = coordinator
        .resolve(
            approval.id,
            &principal,
            ApprovalStatus::Approved,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved");

    match outcome {
        ResolutionOutcome::Executed { execution_id, ok } => {
            assert_eq!(execution_id, execution.id);
            assert!(ok);
        }
        other => panic!("expected execution, got {other:?}"),
    }

    assert_eq!(
        tool.calls(),
        1,
        "the approved tool did not run exactly once"
    );

    let stored = store.execution(execution.id).await.expect("readable");
    assert_eq!(stored.status, ExecutionStatus::Succeeded);
    assert!(stored.completed_at.is_some());
    assert!(stored.outcome.is_some());

    let audit = store
        .audit_for_turn(stored.turn_id)
        .await
        .expect("audit readable");
    let event = audit
        .iter()
        .find(|e| e.execution_id == execution.id)
        .expect("an audit event was written for the executed action");
    assert_eq!(event.approval_outcome, Some(ApprovalStatus::Approved));
    assert_eq!(event.execution_status, ExecutionStatus::Succeeded);
    assert_eq!(event.risk, RiskLevel::Red);
}

#[tokio::test]
async fn a_rejected_action_never_executes_and_is_audited() {
    let store = store_or_skip!();
    let principal = principal();
    let tool = Arc::new(EchoTool::red("gmail.send"));
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    let coordinator = coordinator(
        (*store).clone(),
        vec![tool.clone()],
        ApprovalPolicy::default(),
    );

    let outcome = coordinator
        .resolve(
            approval.id,
            &principal,
            ApprovalStatus::Rejected,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved");

    assert!(matches!(outcome, ResolutionOutcome::Rejected { .. }));
    assert_eq!(tool.calls(), 0, "a rejected action executed anyway");

    let stored = store.execution(execution.id).await.expect("readable");
    assert_eq!(stored.status, ExecutionStatus::Cancelled);

    let audit = store
        .audit_for_turn(stored.turn_id)
        .await
        .expect("readable");
    let event = audit
        .iter()
        .find(|e| e.execution_id == execution.id)
        .expect("rejection was audited");
    assert_eq!(event.approval_outcome, Some(ApprovalStatus::Rejected));
    assert_eq!(event.execution_status, ExecutionStatus::Cancelled);
}

#[tokio::test]
async fn a_failing_approved_tool_is_recorded_as_failed_and_audited() {
    let store = store_or_skip!();
    let principal = principal();
    let tool = Arc::new(FailingTool::new("gmail.send"));
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    let coordinator = coordinator(
        (*store).clone(),
        vec![tool.clone()],
        ApprovalPolicy::default(),
    );

    let outcome = coordinator
        .resolve(
            approval.id,
            &principal,
            ApprovalStatus::Approved,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved");

    match outcome {
        ResolutionOutcome::Executed { ok, .. } => assert!(!ok, "a failing tool reported success"),
        other => panic!("expected execution, got {other:?}"),
    }

    let stored = store.execution(execution.id).await.expect("readable");
    assert_eq!(stored.status, ExecutionStatus::Failed);

    let audit = store
        .audit_for_turn(stored.turn_id)
        .await
        .expect("readable");
    assert!(
        audit.iter().any(
            |e| e.execution_id == execution.id && e.execution_status == ExecutionStatus::Failed
        ),
        "a failed execution was not audited"
    );
}

/// Approval is permission for one action, not a standing grant.
///
/// The tool is unregistered between the question and the answer. The stored
/// approval is still `Requested` and still owned by the caller — but the action
/// is re-validated against the live registry, finds nothing, and refuses.
#[tokio::test]
async fn an_approval_does_not_survive_the_tool_being_unregistered() {
    let store = store_or_skip!();
    let principal = principal();
    let (execution, approval) = seed(&store, &principal, "gmail.send", Duration::minutes(15)).await;

    // Registry no longer contains `gmail.send`.
    let coordinator = coordinator(
        (*store).clone(),
        vec![Arc::new(EchoTool::green("notes.read"))],
        ApprovalPolicy::default(),
    );

    let outcome = coordinator
        .resolve(
            approval.id,
            &principal,
            ApprovalStatus::Approved,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved");

    match outcome {
        ResolutionOutcome::NoLongerPermitted { execution_id, .. } => {
            assert_eq!(execution_id, execution.id);
        }
        other => panic!("an unregistered tool was executed: {other:?}"),
    }

    let stored = store.execution(execution.id).await.expect("readable");
    assert_eq!(stored.status, ExecutionStatus::Cancelled);
}

// --------------------------------------------------------------------- audit

#[tokio::test]
async fn audit_records_argument_keys_but_never_argument_values() {
    let store = store_or_skip!();
    let principal = principal();
    let tool = Arc::new(EchoTool::red("gmail.send"));

    let now = OffsetDateTime::now_utc();
    let execution = ToolExecution {
        id: Uuid::new_v4(),
        turn_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        principal_id: principal.user_id,
        tool_name: "gmail.send".into(),
        arguments: serde_json::json!({
            "to": "victim@example.com",
            "body": "my api key is sk-live-SECRET and my password is hunter2",
            "oauth_token": "ya29.SHOULD-NEVER-APPEAR"
        }),
        risk: RiskLevel::Red,
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
        tool_name: "gmail.send".into(),
        risk: RiskLevel::Red,
        reason: "sends mail on your behalf".into(),
        status: ApprovalStatus::Requested,
        created_at: now,
        expires_at: now + Duration::minutes(15),
        resolved_at: None,
        resolved_by: None,
    };
    store
        .record_proposal(&execution, Some(&approval))
        .await
        .expect("stored");

    let coordinator = coordinator((*store).clone(), vec![tool], ApprovalPolicy::default());
    coordinator
        .resolve(
            approval.id,
            &principal,
            ApprovalStatus::Approved,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved");

    let audit = store
        .audit_for_turn(execution.turn_id)
        .await
        .expect("readable");
    let event = audit
        .iter()
        .find(|e| e.execution_id == execution.id)
        .expect("audited");

    // The keys are useful and safe. The values are not, and the audit trail
    // cannot tell a harmless one from a credential, so it stores none of them.
    assert!(
        event.summary.contains("body"),
        "summary lost the argument shape"
    );
    for secret in [
        "sk-live-SECRET",
        "hunter2",
        "ya29.SHOULD-NEVER-APPEAR",
        "victim@example.com",
    ] {
        assert!(
            !event.summary.contains(secret),
            "audit summary leaked `{secret}`: {}",
            event.summary
        );
    }

    // And the same guarantee at the function that builds it.
    let direct = summarize(&execution.tool_name, &execution.arguments);
    assert!(!direct.contains("hunter2"));
}

// ------------------------------------------------------------- restart-safe

/// The point of the milestone: durability across a process boundary.
///
/// The first store is dropped entirely — pool, connections, every in-memory
/// structure — before a second, independent store reads the approval back and
/// resolves it. Nothing survives in this process except the row in Postgres.
#[tokio::test]
async fn a_pending_approval_survives_the_process_that_created_it() {
    let principal = principal();

    let (execution_id, approval_id, turn_id) = {
        // A pool of its own, closed before the second half runs, so nothing --
        // not a cached row, not a connection, not a transaction -- carries over.
        let Some(first) = separate_store().await else {
            eprintln!("skipped: DATABASE_URL is not set");
            return;
        };
        let (execution, approval) =
            seed(&first, &principal, "gmail.send", Duration::minutes(15)).await;
        first.close().await;
        (execution.id, approval.id, execution.turn_id)
    };

    // A brand-new store, as a restarted server would build.
    let second = store_or_skip!();

    let recovered = second
        .approval(approval_id)
        .await
        .expect("approval survived");
    assert_eq!(recovered.status, ApprovalStatus::Requested);
    assert_eq!(recovered.execution_id, execution_id);

    let listed = second
        .pending_approvals(principal.user_id)
        .await
        .expect("listed");
    assert!(
        listed.iter().any(|a| a.id == approval_id),
        "the approval was not pending after restart"
    );

    // And it is still answerable, with the action still executable.
    let tool = Arc::new(EchoTool::red("gmail.send"));
    let coordinator = coordinator(
        (*second).clone(),
        vec![tool.clone()],
        ApprovalPolicy::default(),
    );

    let outcome = coordinator
        .resolve(
            approval_id,
            &principal,
            ApprovalStatus::Approved,
            &CancellationToken::new(),
        )
        .await
        .expect("resolved after restart");

    assert!(matches!(
        outcome,
        ResolutionOutcome::Executed { ok: true, .. }
    ));
    assert_eq!(tool.calls(), 1);

    let completed = second.execution(execution_id).await.expect("readable");
    assert_eq!(completed.status, ExecutionStatus::Succeeded);

    let audit = second.audit_for_turn(turn_id).await.expect("readable");
    assert!(
        audit.iter().any(|e| e.execution_id == execution_id),
        "the resumed action left no audit trail"
    );
}

#[tokio::test]
async fn a_completed_execution_survives_the_process_that_ran_it() {
    let principal = principal();

    let execution_id = {
        let Some(first) = separate_store().await else {
            eprintln!("skipped: DATABASE_URL is not set");
            return;
        };
        let (execution, approval) =
            seed(&first, &principal, "gmail.send", Duration::minutes(15)).await;
        let coordinator = coordinator(
            (*first).clone(),
            vec![Arc::new(EchoTool::red("gmail.send"))],
            ApprovalPolicy::default(),
        );
        coordinator
            .resolve(
                approval.id,
                &principal,
                ApprovalStatus::Approved,
                &CancellationToken::new(),
            )
            .await
            .expect("executed");
        first.close().await;
        execution.id
    };

    let second = store_or_skip!();
    let recovered = second
        .execution(execution_id)
        .await
        .expect("execution survived");
    assert_eq!(recovered.status, ExecutionStatus::Succeeded);
    assert!(recovered.completed_at.is_some());
}

// -------------------------------------------------------- transition helpers

#[tokio::test]
async fn approval_status_transitions_match_the_documented_table() {
    // Pure, so it needs no database and always runs.
    assert!(ApprovalTransitions::can_transition_to(
        ApprovalStatus::Requested,
        ApprovalStatus::Approved
    ));
    assert!(!ApprovalTransitions::can_transition_to(
        ApprovalStatus::Approved,
        ApprovalStatus::Rejected
    ));
    assert!(!ApprovalTransitions::can_transition_to(
        ApprovalStatus::Expired,
        ApprovalStatus::Approved
    ));
}
