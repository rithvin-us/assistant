//! Conversation persistence tests against a real PostgreSQL.
//!
//! These exercise the actual `PostgresConversationStore` -- its transactions,
//! its ownership-scoped SQL and the table's own CHECK constraints -- because
//! the properties under test are properties of the database. An in-memory fake
//! would pass while proving nothing: "a conversation belonging to another user
//! is indistinguishable from one that does not exist" is a claim about a WHERE
//! clause, and "history survives a restart" is a claim about durable storage.
//!
//! Every test is skipped, not failed, when `DATABASE_URL` is absent, so
//! `cargo test --workspace` stays runnable without credentials. Run them with:
//!
//! ```powershell
//! $env:DATABASE_URL = "<connection string>"; cargo test -p assistant-server --test conversations
//! ```
//!
//! Each test uses a fresh principal and a fresh conversation id, so runs are
//! independent and can execute concurrently against one database.

use std::sync::Arc;

use assistant_core::{
    ContextProvider, ContextWindow, MessageRole, NewMessage, Orchestrator, StoredContextProvider,
    conversation::ConversationStore, testing::dev_principal, turn::TurnRequest,
};
use assistant_models::mock::{MockModelProvider, MockResponse};
use assistant_server::conversations::PostgresConversationStore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Caps how many tests may hold database connections at once.
///
/// A hosted Supabase session pooler admits a fixed number of clients and
/// refuses the rest. The test harness runs tests in parallel with no idea about
/// that, so the limit is enforced here; without it the suite fails on
/// connection admission rather than on anything it is trying to assert.
static DB_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Opens a pool for one test.
///
/// Deliberately *not* shared across tests: each `#[tokio::test]` builds its own
/// runtime, and a `sqlx` pool is bound to the runtime that created it.
async fn pool_with(max_connections: u32) -> Option<sqlx::PgPool> {
    let _ = dotenvy::dotenv();
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.is_empty())?;

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
struct TestStore {
    store: Arc<PostgresConversationStore>,
    _permit: tokio::sync::SemaphorePermit<'static>,
}

impl std::ops::Deref for TestStore {
    type Target = Arc<PostgresConversationStore>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

async fn store() -> Option<TestStore> {
    let permit = DB_PERMITS.acquire().await.expect("semaphore open");
    let pool = pool_with(1).await?;
    Some(TestStore {
        store: Arc::new(PostgresConversationStore::new(pool)),
        _permit: permit,
    })
}

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

fn principal() -> Uuid {
    // A fresh principal per test namespaces every row it writes.
    Uuid::new_v4()
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_conversation_is_created_once_and_resolving_it_again_is_idempotent() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();

    let first = store.ensure(id, owner).await.expect("created");
    assert_eq!(first.id, id);
    assert_eq!(first.principal_id, owner);

    let second = store.ensure(id, owner).await.expect("resolved");
    assert_eq!(second.created_at, first.created_at, "the row was replaced");
}

#[tokio::test]
async fn messages_come_back_in_the_order_they_were_written() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();
    let turn = Uuid::new_v4();
    store.ensure(id, owner).await.expect("created");

    for text in ["first", "second", "third", "fourth"] {
        store
            .append(&NewMessage::user(id, owner, turn, text))
            .await
            .expect("appended");
    }

    let history = store.history(id, owner, 10).await.expect("read");
    let contents: Vec<&str> = history
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(contents, ["first", "second", "third", "fourth"]);

    // `seq` is strictly increasing, which is what the ordering relies on.
    assert!(history.windows(2).all(|pair| pair[0].seq < pair[1].seq));
}

#[tokio::test]
async fn the_limit_returns_the_most_recent_messages_not_the_oldest() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();
    let turn = Uuid::new_v4();
    store.ensure(id, owner).await.expect("created");

    for index in 0..6 {
        store
            .append(&NewMessage::user(id, owner, turn, format!("m{index}")))
            .await
            .expect("appended");
    }

    let history = store.history(id, owner, 2).await.expect("read");
    let contents: Vec<&str> = history
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(contents, ["m4", "m5"]);
}

#[tokio::test]
async fn every_role_round_trips_including_its_structured_tool_data() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();
    let turn = Uuid::new_v4();
    store.ensure(id, owner).await.expect("created");

    let call = assistant_tools::ToolCall {
        id: "toolu_1".into(),
        name: "notes.read".into(),
        arguments: serde_json::json!({"limit": 5}),
    };

    store
        .append(&NewMessage::user(id, owner, turn, "read my notes"))
        .await
        .expect("appended");
    store
        .append(
            &NewMessage::assistant(id, owner, turn, "Checking.")
                .with_tool_calls(vec![call.clone()]),
        )
        .await
        .expect("appended");
    store
        .append(&NewMessage::tool(
            id,
            owner,
            turn,
            "toolu_1",
            r#"{"notes":[]}"#,
        ))
        .await
        .expect("appended");

    let history = store.history(id, owner, 10).await.expect("read");
    assert_eq!(
        history
            .iter()
            .map(|message| message.role)
            .collect::<Vec<_>>(),
        [MessageRole::User, MessageRole::Assistant, MessageRole::Tool]
    );

    assert_eq!(history[1].tool_calls.len(), 1);
    assert_eq!(history[1].tool_calls[0].name, "notes.read");
    assert_eq!(history[1].tool_calls[0].arguments["limit"], 5);
    assert_eq!(history[2].tool_call_id.as_deref(), Some("toolu_1"));
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_conversation_owned_by_someone_else_looks_exactly_like_one_that_does_not_exist() {
    let store = store_or_skip!();
    let owner = principal();
    let intruder = principal();
    let id = Uuid::new_v4();

    store.ensure(id, owner).await.expect("created");
    store
        .append(&NewMessage::user(id, owner, Uuid::new_v4(), "private"))
        .await
        .expect("appended");

    // Resolving it as the wrong principal must not adopt it, and must not
    // report a distinguishable "forbidden".
    let taken = store.ensure(id, intruder).await;
    let missing = store.ensure(Uuid::new_v4(), intruder).await;
    assert!(matches!(
        taken,
        Err(assistant_core::ConversationError::NotFound(_))
    ));
    assert!(missing.is_ok(), "a genuinely new id is simply created");

    // Owner is unchanged.
    let still_mine = store.ensure(id, owner).await.expect("resolved");
    assert_eq!(still_mine.principal_id, owner);

    // Neither reading nor writing crosses the boundary.
    assert!(
        store
            .history(id, intruder, 10)
            .await
            .expect("read")
            .is_empty(),
        "history leaked to another principal"
    );
    assert!(
        store
            .append(&NewMessage::user(id, intruder, Uuid::new_v4(), "injected"))
            .await
            .is_err(),
        "another principal wrote into this conversation"
    );

    // And the owner's history is untouched by the attempt.
    let history = store.history(id, owner, 10).await.expect("read");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "private");
}

#[tokio::test]
async fn writing_to_a_conversation_that_was_never_created_fails_rather_than_creating_one() {
    let store = store_or_skip!();
    let owner = principal();

    let result = store
        .append(&NewMessage::user(
            Uuid::new_v4(),
            owner,
            Uuid::new_v4(),
            "orphan",
        ))
        .await;

    assert!(matches!(
        result,
        Err(assistant_core::ConversationError::NotFound(_))
    ));
}

// ---------------------------------------------------------------------------
// Schema guarantees
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_database_refuses_a_user_message_that_carries_tool_calls() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();
    store.ensure(id, owner).await.expect("created");

    // A user-supplied tool call is the shape an injection attempt would take,
    // so the constraint is in the schema rather than only in Rust.
    let mut message = NewMessage::user(id, owner, Uuid::new_v4(), "please");
    message.tool_calls = vec![assistant_tools::ToolCall {
        id: "toolu_x".into(),
        name: "gmail.send".into(),
        arguments: serde_json::json!({}),
    }];

    assert!(
        store.append(&message).await.is_err(),
        "the database accepted tool calls on a user message"
    );
}

// ---------------------------------------------------------------------------
// Survival across a restart
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_conversation_survives_the_process_that_created_it() {
    let owner = principal();
    let id = Uuid::new_v4();

    // "Before the restart": a store, a pool and an orchestrator that all go out
    // of scope at the end of this block, exactly as they would when a process
    // exits.
    {
        let store = store_or_skip!();
        let model = Arc::new(MockModelProvider::new(vec![MockResponse::text("Noted.")]));
        let orchestrator = Arc::new(
            Orchestrator::builder()
                .model(model)
                .conversations(store.store.clone())
                .context(Arc::new(StoredContextProvider::new(
                    store.store.clone(),
                    ContextWindow::default(),
                )))
                .build(),
        );

        let mut principal = dev_principal();
        principal.user_id = owner;

        orchestrator
            .run(
                TurnRequest::new(id, principal, "My name is Alex."),
                CancellationToken::new(),
            )
            .await
            .expect("answered");
    }

    // "After the restart": a brand new pool, store and orchestrator. Nothing is
    // carried over in memory; if the history comes back, it came from Postgres.
    let store = store_or_skip!();
    let model = Arc::new(MockModelProvider::new(vec![MockResponse::text(
        "Your name is Alex.",
    )]));
    let orchestrator = Arc::new(
        Orchestrator::builder()
            .model(model.clone())
            .conversations(store.store.clone())
            .context(Arc::new(StoredContextProvider::new(
                store.store.clone(),
                ContextWindow::default(),
            )))
            .build(),
    );

    let mut principal = dev_principal();
    principal.user_id = owner;

    let outcome = orchestrator
        .run(
            TurnRequest::new(id, principal, "What is my name?"),
            CancellationToken::new(),
        )
        .await
        .expect("answered");

    assert_eq!(outcome.text, "Your name is Alex.");

    let replayed: Vec<String> = model.requests()[0]
        .messages
        .iter()
        .map(|message| message.content.clone())
        .collect();
    assert_eq!(
        replayed,
        [
            "My name is Alex.".to_string(),
            "Noted.".to_string(),
            "What is my name?".to_string(),
        ],
        "the conversation did not survive the restart"
    );
}

#[tokio::test]
async fn a_failed_turn_persists_the_question_and_no_answer() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();

    let model = Arc::new(MockModelProvider::new(vec![MockResponse::Error(
        "overloaded".into(),
    )]));
    let orchestrator = Orchestrator::builder()
        .model(model)
        .conversations(store.store.clone())
        .context(Arc::new(StoredContextProvider::new(
            store.store.clone(),
            ContextWindow::default(),
        )))
        .build();

    let mut principal = dev_principal();
    principal.user_id = owner;

    let error = orchestrator
        .run(
            TurnRequest::new(id, principal, "did it work"),
            CancellationToken::new(),
        )
        .await
        .expect_err("failed");
    assert_eq!(error.code(), "provider_error");

    let history = store.history(id, owner, 10).await.expect("read");
    assert_eq!(
        history.len(),
        1,
        "a failed turn wrote an answer: {history:?}"
    );
    assert_eq!(history[0].role, MessageRole::User);
}

#[tokio::test]
async fn the_context_provider_reads_through_to_postgres_within_its_window() {
    let store = store_or_skip!();
    let owner = principal();
    let id = Uuid::new_v4();
    let turn = Uuid::new_v4();
    store.ensure(id, owner).await.expect("created");

    for index in 0..8 {
        store
            .append(&NewMessage::user(id, owner, turn, format!("m{index}")))
            .await
            .expect("appended");
    }

    let provider = StoredContextProvider::new(
        store.store.clone(),
        ContextWindow {
            max_messages: 3,
            max_chars: 10_000,
        },
    );

    let mut principal = dev_principal();
    principal.user_id = owner;

    let context = provider
        .assemble(&TurnRequest::new(id, principal, "next"))
        .await
        .expect("assembled");

    let contents: Vec<&str> = context
        .history
        .iter()
        .map(|entry| entry.content.as_str())
        .collect();
    assert_eq!(contents, ["m5", "m6", "m7"]);
}
