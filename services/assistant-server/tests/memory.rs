//! Memory persistence and API tests against a real PostgreSQL.
//!
//! These exercise the actual `PostgresMemoryStore` and the memory routes in
//! `services/assistant-server/src/routes/memory.rs`, because the properties
//! under test -- ownership isolation, the temporary/expiry constraint,
//! secret-like content refusal, lifecycle transitions, and the ranked
//! retrieval order -- are properties of the SQL and the router. An in-memory
//! fake would pass while proving nothing.
//!
//! Every test is skipped, not failed, when `DATABASE_URL` is absent, so
//! `cargo test --workspace` stays runnable without credentials. Run them with:
//!
//! ```powershell
//! $env:DATABASE_URL = "<connection string>"; cargo test -p assistant-server --test memory
//! ```
//!
//! Each test namespaces its rows by a fresh principal uuid, so runs are
//! independent and can execute concurrently against one database.

use std::{net::SocketAddr, sync::Arc};

use assistant_core::{EventBus, ToolRegistry};
use assistant_memory::{
    Confidence, Importance, Lifecycle, MemoryKind, MemoryPatch, MemoryQuery, MemorySource,
    MemoryStore, NewMemory, Provenance,
};
use assistant_protocol::{
    CreateMemoryRequest, MemoryItem, MemoryKindDto, MemoryLifecycleDto, MemorySourceDto,
    UpdateMemoryRequest,
};
use assistant_server::{
    app, config::Config, memory_store::PostgresMemoryStore, orchestration::Dependencies,
};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

const TEST_TOKEN: &str = "memory-integration-token";

static DB_PERMITS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

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

struct TestStore {
    store: Arc<PostgresMemoryStore>,
    _permit: tokio::sync::SemaphorePermit<'static>,
}

impl std::ops::Deref for TestStore {
    type Target = Arc<PostgresMemoryStore>;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

async fn store() -> Option<TestStore> {
    let permit = DB_PERMITS.acquire().await.expect("semaphore open");
    let pool = pool_with(1).await?;
    Some(TestStore {
        store: Arc::new(PostgresMemoryStore::new(pool)),
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

fn user() -> Uuid {
    Uuid::new_v4()
}

// ---------------------------------------------------------------------------
// Store-level tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ownership_is_enforced_by_sql_not_by_a_rust_side_check() {
    let store = store_or_skip!();
    let alice = user();
    let bob = user();

    let alice_memory = store
        .create(NewMemory::explicit(
            alice,
            MemoryKind::Fact,
            "alice loves tea",
        ))
        .await
        .expect("stored");

    // Bob's principal must not see the row, and must not be able to change it.
    assert!(matches!(
        store.get(bob, alice_memory.id).await,
        Err(assistant_memory::MemoryError::NotFound(_))
    ));
    assert!(matches!(
        store.archive(bob, alice_memory.id).await,
        Err(assistant_memory::MemoryError::NotFound(_))
    ));
    let bob_hits = store
        .search(MemoryQuery::active(bob))
        .await
        .expect("searched");
    assert!(bob_hits.is_empty());
}

#[tokio::test]
async fn temporary_memory_expiry_is_enforced_by_a_check_constraint() {
    let store = store_or_skip!();
    let uid = user();

    let no_expiry = NewMemory {
        user_id: uid,
        kind: MemoryKind::Temporary,
        content: "tonight only".into(),
        importance: Importance::NORMAL,
        confidence: Confidence::CERTAIN,
        provenance: Provenance::explicit(),
        expires_at: None,
    };
    assert!(matches!(
        store.create(no_expiry).await,
        Err(assistant_memory::MemoryError::Invalid(_))
    ));

    let non_temp_with_expiry = NewMemory {
        user_id: uid,
        kind: MemoryKind::Fact,
        content: "the sky is blue".into(),
        importance: Importance::NORMAL,
        confidence: Confidence::CERTAIN,
        provenance: Provenance::explicit(),
        expires_at: Some(OffsetDateTime::now_utc() + time::Duration::hours(1)),
    };
    assert!(matches!(
        store.create(non_temp_with_expiry).await,
        Err(assistant_memory::MemoryError::Invalid(_))
    ));
}

#[tokio::test]
async fn secret_like_content_is_refused_before_it_reaches_the_database() {
    let store = store_or_skip!();
    let uid = user();
    let bad = NewMemory::explicit(
        uid,
        MemoryKind::Fact,
        "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.abc",
    );
    assert!(matches!(
        store.create(bad).await,
        Err(assistant_memory::MemoryError::SecretLike)
    ));

    let after = store
        .search(MemoryQuery::active(uid))
        .await
        .expect("searched");
    assert!(after.is_empty(), "secret memory was persisted");
}

#[tokio::test]
async fn archive_and_restore_move_the_lifecycle_and_the_timestamp() {
    let store = store_or_skip!();
    let uid = user();
    let stored = store
        .create(NewMemory::explicit(uid, MemoryKind::Fact, "hi"))
        .await
        .unwrap();
    let archived = store.archive(uid, stored.id).await.expect("archived");
    assert_eq!(archived.lifecycle, Lifecycle::Archived);
    assert!(archived.archived_at.is_some());

    let restored = store.restore(uid, stored.id).await.expect("restored");
    assert_eq!(restored.lifecycle, Lifecycle::Active);
    assert!(restored.archived_at.is_none());
}

#[tokio::test]
async fn supersede_marks_the_old_row_and_points_it_at_the_new() {
    let store = store_or_skip!();
    let uid = user();
    let old = store
        .create(NewMemory::explicit(
            uid,
            MemoryKind::Preference,
            "I prefer Python",
        ))
        .await
        .unwrap();
    let new_memory = store
        .create(NewMemory::explicit(
            uid,
            MemoryKind::Preference,
            "I prefer Rust",
        ))
        .await
        .unwrap();
    store.supersede(uid, old.id, new_memory.id).await.unwrap();

    let refreshed = store.get(uid, old.id).await.unwrap();
    assert_eq!(refreshed.lifecycle, Lifecycle::Superseded);
    assert_eq!(refreshed.superseded_by, Some(new_memory.id));

    // Active search excludes superseded by default.
    let active = store.search(MemoryQuery::active(uid)).await.unwrap();
    assert!(active.iter().all(|memory| memory.id != old.id));
}

#[tokio::test]
async fn supersede_refuses_across_owners() {
    let store = store_or_skip!();
    let alice = user();
    let bob = user();

    let alice_old = store
        .create(NewMemory::explicit(alice, MemoryKind::Fact, "alice"))
        .await
        .unwrap();
    let bob_new = store
        .create(NewMemory::explicit(bob, MemoryKind::Fact, "bob"))
        .await
        .unwrap();

    // Alice cannot mark her memory as superseded by Bob's memory.
    let outcome = store.supersede(alice, alice_old.id, bob_new.id).await;
    assert!(matches!(
        outcome,
        Err(assistant_memory::MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn temporary_memories_sweep_when_expired() {
    let store = store_or_skip!();
    let uid = user();
    let expiring = NewMemory::explicit(uid, MemoryKind::Temporary, "tonight")
        .with_expiry(OffsetDateTime::now_utc() - time::Duration::seconds(1));
    let stored = store.create(expiring).await.unwrap();

    let moved = store
        .sweep_expired(uid, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert!(moved >= 1);
    let refreshed = store.get(uid, stored.id).await.unwrap();
    assert_eq!(refreshed.lifecycle, Lifecycle::Archived);
}

#[tokio::test]
async fn touch_updates_access_metadata_only_when_called() {
    let store = store_or_skip!();
    let uid = user();
    let stored = store
        .create(NewMemory::explicit(uid, MemoryKind::Fact, "hi"))
        .await
        .unwrap();
    assert_eq!(stored.access_count, 0);
    assert!(stored.last_accessed_at.is_none());

    // A search alone does not count as an access.
    let _ = store.search(MemoryQuery::active(uid)).await.unwrap();
    let after_search = store.get(uid, stored.id).await.unwrap();
    assert_eq!(after_search.access_count, 0);

    let now = OffsetDateTime::now_utc();
    store.touch(uid, &[stored.id], now).await.unwrap();
    let after_touch = store.get(uid, stored.id).await.unwrap();
    assert_eq!(after_touch.access_count, 1);
    assert!(after_touch.last_accessed_at.is_some());
}

#[tokio::test]
async fn search_ranks_text_hits_above_recent_misses() {
    let store = store_or_skip!();
    let uid = user();
    let miss = store
        .create(NewMemory::explicit(
            uid,
            MemoryKind::Fact,
            "the sky is blue today",
        ))
        .await
        .unwrap();
    let hit = store
        .create(NewMemory::explicit(
            uid,
            MemoryKind::Preference,
            "I prefer cats over dogs",
        ))
        .await
        .unwrap();
    let hits = store
        .search(MemoryQuery {
            user_id: uid,
            text: Some("cats".into()),
            lifecycles: vec![Lifecycle::Active],
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        hits.iter().any(|memory| memory.id == hit.id),
        "text hit missing"
    );
    // The miss should either be excluded or ranked below the hit.
    if let Some(pos_hit) = hits.iter().position(|memory| memory.id == hit.id)
        && let Some(pos_miss) = hits.iter().position(|memory| memory.id == miss.id)
    {
        assert!(pos_hit < pos_miss, "text hit was ranked below a miss");
    }
}

#[tokio::test]
async fn search_enforces_a_bounded_result_count() {
    let store = store_or_skip!();
    let uid = user();
    for i in 0..20 {
        store
            .create(NewMemory::explicit(
                uid,
                MemoryKind::Fact,
                format!("fact number {i}"),
            ))
            .await
            .unwrap();
    }
    let hits = store
        .search(MemoryQuery {
            user_id: uid,
            limit: 5,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 5);
}

#[tokio::test]
async fn update_patch_leaves_unnamed_fields_alone() {
    let store = store_or_skip!();
    let uid = user();
    let stored = store
        .create(NewMemory::explicit(uid, MemoryKind::Fact, "hello world"))
        .await
        .unwrap();

    let patched = store
        .update(
            uid,
            stored.id,
            MemoryPatch {
                importance: Some(Importance::MAX),
                ..Default::default()
            },
        )
        .await
        .expect("patched");
    assert_eq!(patched.importance, Importance::MAX);
    assert_eq!(patched.content, "hello world");
    assert_eq!(patched.kind, MemoryKind::Fact);
    assert_eq!(
        patched.provenance.source_kind,
        MemorySource::ExplicitUserInput
    );
}

#[tokio::test]
async fn update_refuses_secret_like_content() {
    let store = store_or_skip!();
    let uid = user();
    let stored = store
        .create(NewMemory::explicit(uid, MemoryKind::Fact, "innocent fact"))
        .await
        .unwrap();
    let outcome = store
        .update(
            uid,
            stored.id,
            MemoryPatch {
                content: Some("sk-abcdefghijklmnop1234567890".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(
        outcome,
        Err(assistant_memory::MemoryError::SecretLike)
    ));
    let refreshed = store.get(uid, stored.id).await.unwrap();
    assert_eq!(refreshed.content, "innocent fact");
}

// ---------------------------------------------------------------------------
// HTTP-level tests (the router + PostgresMemoryStore)
// ---------------------------------------------------------------------------

fn test_config() -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        database_url: std::env::var("DATABASE_URL").ok(),
        dev_auth_token: TEST_TOKEN.to_string(),
        supabase_project_ref: None,
        allowed_origins: vec!["http://localhost:1420".to_string()],
        log_filter: "off".to_string(),
        max_tool_rounds: 4,
        openai_api_key: None,
        openai_transcription_model: "whisper-1".to_string(),
        openai_transcription_language: None,
        model: "test-model".to_string(),
        model_max_output_tokens: 1024,
        model_timeout: std::time::Duration::from_secs(5),
        context_max_messages: 40,
        google_client_id: None,
        google_client_secret: None,
        google_redirect_uri: None,
        credential_encryption_key: None,
        document_storage_dir: std::path::PathBuf::from("./data/documents"),
    }
}

/// A running test server plus the semaphore permit that entitles it to a
/// database connection. Both are held for the life of the test so the
/// connection cap is honoured even though the router owns the pool internally.
struct TestServer {
    addr: SocketAddr,
    _permit: tokio::sync::SemaphorePermit<'static>,
}

async fn spawn_or_skip() -> Option<TestServer> {
    let _ = dotenvy::dotenv();
    let permit = DB_PERMITS.acquire().await.ok()?;
    let pool = pool_with(2).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = test_config();
    let deps = Dependencies {
        tools: Arc::new(ToolRegistry::new()),
        ..Default::default()
    };
    let router = app(&config, Some(pool), EventBus::default(), deps);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Some(TestServer {
        addr,
        _permit: permit,
    })
}

macro_rules! server_or_skip {
    () => {
        match spawn_or_skip().await {
            Some(server) => server,
            None => {
                eprintln!("skipped: DATABASE_URL is not set");
                return;
            }
        }
    };
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn post_json<T: serde::Serialize>(
    addr: SocketAddr,
    path: &str,
    body: &T,
) -> reqwest::Response {
    client()
        .post(format!("http://{addr}{path}"))
        .bearer_auth(TEST_TOKEN)
        .json(body)
        .send()
        .await
        .expect("post")
}

async fn get(addr: SocketAddr, path: &str) -> reqwest::Response {
    client()
        .get(format!("http://{addr}{path}"))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("get")
}

async fn patch_json<T: serde::Serialize>(
    addr: SocketAddr,
    path: &str,
    body: &T,
) -> reqwest::Response {
    client()
        .patch(format!("http://{addr}{path}"))
        .bearer_auth(TEST_TOKEN)
        .json(body)
        .send()
        .await
        .expect("patch")
}

#[tokio::test]
async fn create_get_and_search_round_trip() {
    let server = server_or_skip!();
    let addr = server.addr;

    let create = CreateMemoryRequest {
        kind: MemoryKindDto::Preference,
        content: "The user prefers concise, technical answers.".into(),
        importance: Some(4),
        confidence: Some(0.95),
        source_kind: Some(MemorySourceDto::ExplicitUserInput),
        source_ref: None,
        expires_at: None,
        supersedes: None,
    };
    let response = post_json(addr, "/v1/memories", &create).await;
    assert!(response.status().is_success(), "{:?}", response.status());
    let stored: MemoryItem = response.json().await.unwrap();
    assert_eq!(stored.kind, MemoryKindDto::Preference);
    assert_eq!(stored.importance, 4);

    let fetched: MemoryItem = get(addr, &format!("/v1/memories/{}", stored.id))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(fetched.id, stored.id);

    let list: Vec<MemoryItem> = get(addr, "/v1/memories?q=concise")
        .await
        .json()
        .await
        .unwrap();
    assert!(list.iter().any(|memory| memory.id == stored.id));
}

#[tokio::test]
async fn create_rejects_secret_like_content_with_400() {
    let server = server_or_skip!();
    let addr = server.addr;
    let body = json!({
        "kind": "fact",
        "content": "api_key=abcdef1234567890abcdef1234567890"
    });
    let response = post_json(addr, "/v1/memories", &body).await;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_rejects_temporary_without_expiry_with_400() {
    let server = server_or_skip!();
    let addr = server.addr;
    let body = json!({
        "kind": "temporary",
        "content": "tonight only"
    });
    let response = post_json(addr, "/v1/memories", &body).await;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn archive_then_restore_round_trip() {
    let server = server_or_skip!();
    let addr = server.addr;
    let create = CreateMemoryRequest {
        kind: MemoryKindDto::Fact,
        content: format!("archive-me-{}", Uuid::new_v4()),
        importance: None,
        confidence: None,
        source_kind: None,
        source_ref: None,
        expires_at: None,
        supersedes: None,
    };
    let stored: MemoryItem = post_json(addr, "/v1/memories", &create)
        .await
        .json()
        .await
        .unwrap();

    let archived: MemoryItem = post_json(
        addr,
        &format!("/v1/memories/{}/archive", stored.id),
        &json!({}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(archived.lifecycle, MemoryLifecycleDto::Archived);

    // Default lifecycle filter excludes archived rows.
    let active: Vec<MemoryItem> = get(addr, "/v1/memories").await.json().await.unwrap();
    assert!(active.iter().all(|memory| memory.id != stored.id));

    // Explicit archived filter includes them.
    let archived_list: Vec<MemoryItem> = get(addr, "/v1/memories?lifecycle=archived")
        .await
        .json()
        .await
        .unwrap();
    assert!(archived_list.iter().any(|memory| memory.id == stored.id));

    let restored: MemoryItem = post_json(
        addr,
        &format!("/v1/memories/{}/restore", stored.id),
        &json!({}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(restored.lifecycle, MemoryLifecycleDto::Active);
}

#[tokio::test]
async fn patch_updates_named_fields_only() {
    let server = server_or_skip!();
    let addr = server.addr;
    let created: MemoryItem = post_json(
        addr,
        "/v1/memories",
        &CreateMemoryRequest {
            kind: MemoryKindDto::Fact,
            content: format!("patch-me-{}", Uuid::new_v4()),
            importance: Some(2),
            confidence: Some(0.5),
            source_kind: None,
            source_ref: None,
            expires_at: None,
            supersedes: None,
        },
    )
    .await
    .json()
    .await
    .unwrap();

    let patched: MemoryItem = patch_json(
        addr,
        &format!("/v1/memories/{}", created.id),
        &UpdateMemoryRequest {
            importance: Some(5),
            ..Default::default()
        },
    )
    .await
    .json()
    .await
    .unwrap();

    assert_eq!(patched.importance, 5);
    assert!((patched.confidence - 0.5).abs() < 1e-6);
    assert_eq!(patched.content, created.content);
}

#[tokio::test]
async fn supersedes_replaces_an_older_memory_atomically() {
    let server = server_or_skip!();
    let addr = server.addr;
    let uniq = Uuid::new_v4();
    let old: MemoryItem = post_json(
        addr,
        "/v1/memories",
        &CreateMemoryRequest {
            kind: MemoryKindDto::Preference,
            content: format!("I prefer Python {uniq}"),
            importance: None,
            confidence: None,
            source_kind: None,
            source_ref: None,
            expires_at: None,
            supersedes: None,
        },
    )
    .await
    .json()
    .await
    .unwrap();

    let new: MemoryItem = post_json(
        addr,
        "/v1/memories",
        &CreateMemoryRequest {
            kind: MemoryKindDto::Preference,
            content: format!("I prefer Rust {uniq}"),
            importance: None,
            confidence: None,
            source_kind: None,
            source_ref: None,
            expires_at: None,
            supersedes: Some(old.id),
        },
    )
    .await
    .json()
    .await
    .unwrap();

    let refreshed_old: MemoryItem = get(addr, &format!("/v1/memories/{}", old.id))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(refreshed_old.lifecycle, MemoryLifecycleDto::Superseded);
    assert_eq!(refreshed_old.superseded_by, Some(new.id));
}

#[tokio::test]
async fn unauthenticated_requests_are_refused() {
    let server = server_or_skip!();
    let addr = server.addr;
    let response = reqwest::Client::new()
        .get(format!("http://{addr}/v1/memories"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Protocol serialisation smoke tests
// ---------------------------------------------------------------------------

#[test]
fn memory_dto_round_trips_through_serde() {
    let memory = MemoryItem {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        kind: MemoryKindDto::Preference,
        lifecycle: MemoryLifecycleDto::Active,
        content: "test".into(),
        importance: 4,
        confidence: 0.8,
        provenance: assistant_protocol::MemoryProvenanceDto {
            source_kind: MemorySourceDto::Conversation,
            source_ref: Some("conv-1".into()),
        },
        expires_at: None,
        created_at: OffsetDateTime::now_utc(),
        updated_at: OffsetDateTime::now_utc(),
        last_accessed_at: None,
        access_count: 3,
        archived_at: None,
        superseded_by: None,
    };
    let serialized = serde_json::to_string(&memory).expect("serialize");
    let deserialized: MemoryItem = serde_json::from_str(&serialized).expect("deserialize");
    assert_eq!(deserialized.id, memory.id);
    assert_eq!(deserialized.kind, MemoryKindDto::Preference);
    assert_eq!(deserialized.lifecycle, MemoryLifecycleDto::Active);
}

#[test]
fn create_memory_request_accepts_minimal_shape() {
    let body = json!({
        "kind": "fact",
        "content": "the sky is blue"
    });
    let parsed: CreateMemoryRequest = serde_json::from_value(body).expect("parsed");
    assert_eq!(parsed.kind, MemoryKindDto::Fact);
    assert!(parsed.importance.is_none());
    assert!(parsed.supersedes.is_none());
}
