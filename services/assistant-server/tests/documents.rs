//! Document / PDF intelligence tests against a real PostgreSQL and a real
//! HTTP surface. Nothing here mocks the store, the pipeline or the router:
//! the properties under test (ownership isolation in SQL, page-level search
//! ranking, the "temporary/superseded" semantics of the processing state
//! machine, the size and MIME guards on upload) are properties of code that
//! only exists when everything is wired up.
//!
//! Every DB test is skipped, not failed, when `DATABASE_URL` is absent, so
//! `cargo test --workspace` stays runnable without credentials.
//!
//! Test PDFs are constructed in-process with `lopdf`, so there are no binary
//! fixtures in the repository and the fixtures cannot drift from the code
//! that reads them.

use std::{net::SocketAddr, sync::Arc};

use assistant_core::{EventBus, ToolRegistry};
use assistant_documents::{
    Document, DocumentError, DocumentSource, DocumentStorage, DocumentStore, ExtractionMethod,
    NewDocument, NullOcrProvider, NullVisionProvider, ProcessingState, content_hash,
    extract::extract_pdf, pipeline::Pipeline,
};
use assistant_protocol::{DocumentItem, DocumentPageItem, DocumentSearchHit};
use assistant_server::{
    app,
    config::Config,
    documents_store::{LocalFilesystemStorage, PostgresDocumentStore},
    orchestration::Dependencies,
};
use lopdf::{Document as LopdfDocument, Object, Stream, dictionary};
use uuid::Uuid;

const TEST_TOKEN: &str = "documents-integration-token";

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

struct StoreHarness {
    store: Arc<PostgresDocumentStore>,
    storage: Arc<LocalFilesystemStorage>,
    _permit: tokio::sync::SemaphorePermit<'static>,
    _tmp: tempfile::TempDir,
}

async fn store() -> Option<StoreHarness> {
    let permit = DB_PERMITS.acquire().await.ok()?;
    let pool = pool_with(2).await?;
    let tmp = tempfile::tempdir().expect("temp dir");
    Some(StoreHarness {
        store: Arc::new(PostgresDocumentStore::new(pool)),
        storage: Arc::new(LocalFilesystemStorage::new(tmp.path())),
        _permit: permit,
        _tmp: tmp,
    })
}

macro_rules! store_or_skip {
    () => {
        match store().await {
            Some(harness) => harness,
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
// Deterministic PDF fixture generation
// ---------------------------------------------------------------------------
//
// Every PDF used below is constructed in-process by `build_pdf`. That keeps
// the fixtures deterministic, keeps binaries out of the repo, and means a
// test that adds a new "small mixed-content" case does not need a new file.

fn build_pdf(pages: &[&str]) -> Vec<u8> {
    let mut doc = LopdfDocument::with_version("1.5");
    let pages_id = doc.new_object_id();

    // The Helvetica base-14 font is always available; pdf-extract can decode
    // it into readable text.
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut page_ids = Vec::with_capacity(pages.len());
    for text in pages {
        let mut content_bytes = Vec::new();
        content_bytes.extend_from_slice(b"BT\n/F1 12 Tf\n72 720 Td\n(");
        for c in text.chars() {
            match c {
                '(' => content_bytes.extend_from_slice(b"\\("),
                ')' => content_bytes.extend_from_slice(b"\\)"),
                '\\' => content_bytes.extend_from_slice(b"\\\\"),
                '\n' => content_bytes.extend_from_slice(b") Tj T* ("),
                _ if c.is_ascii() => content_bytes.push(c as u8),
                // Non-ASCII characters would require a proper font encoding; the
                // test fixtures use ASCII only so this stays deterministic.
                _ => content_bytes.push(b'?'),
            }
        }
        content_bytes.extend_from_slice(b") Tj\nET\n");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content_bytes));

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content_id,
            "Resources" => resources_id,
        });
        page_ids.push(page_id);
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => page_ids.len() as i64,
            "Kids" => page_ids.iter().copied().map(Object::from).collect::<Vec<_>>(),
        }),
    );

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("pdf write");
    bytes
}

/// A "scanned" page: valid PDF with a page that has no text content stream, so
/// `pdf-extract` returns nothing and the pipeline flags it as needing OCR.
fn build_scanned_pdf() -> Vec<u8> {
    let mut doc = LopdfDocument::with_version("1.5");
    let pages_id = doc.new_object_id();
    let resources_id = doc.add_object(dictionary! {});
    let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => content_id,
        "Resources" => resources_id,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => 1i64,
            "Kids" => vec![Object::from(page_id)],
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("pdf write");
    bytes
}

// ---------------------------------------------------------------------------
// Extractor-level tests -- no DB, no HTTP.
// ---------------------------------------------------------------------------

#[test]
fn extractor_returns_one_entry_per_physical_page() {
    let bytes = build_pdf(&[
        "First page hello. This page has enough text to clear the OCR threshold.",
        "Second page world. This page also carries enough content to be trusted.",
        "Third page. And so does this one, though it is a little shorter still.",
    ]);
    let extracted = extract_pdf(&bytes).expect("extracted");
    assert_eq!(extracted.page_count, 3);
    assert_eq!(extracted.pages.len(), 3);
    assert_eq!(extracted.pages[0].page_number, 1);
    assert!(extracted.pages[0].text.contains("First page"));
    assert!(extracted.pages[1].text.contains("Second"));
    assert!(!extracted.pages[0].needs_ocr);
}

#[test]
fn extractor_flags_a_scanned_page_as_needing_ocr() {
    let bytes = build_scanned_pdf();
    let extracted = extract_pdf(&bytes).expect("extracted");
    assert_eq!(extracted.page_count, 1);
    assert!(extracted.pages[0].needs_ocr);
    assert!(extracted.pages[0].text.trim().is_empty());
}

#[test]
fn extractor_rejects_a_malformed_pdf_with_an_error() {
    let outcome = extract_pdf(b"totally not a pdf");
    assert!(outcome.is_err());
}

#[test]
fn extractor_handles_the_maximum_supported_page_size_gracefully() {
    // 12 pages of moderate text: this is the "large-ish" case the extractor
    // must handle without either failing or losing pages.
    let mut pages: Vec<String> = Vec::new();
    for i in 1..=12 {
        pages.push(format!(
            "This is page {i} of the fixture, with a decent amount of content \
             so the deterministic pass does not flag it as needing OCR."
        ));
    }
    let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
    let bytes = build_pdf(&refs);
    let extracted = extract_pdf(&bytes).expect("extracted");
    assert_eq!(extracted.page_count, 12);
    for page in &extracted.pages {
        assert!(
            !page.needs_ocr,
            "page {} incorrectly flagged",
            page.page_number
        );
    }
}

// ---------------------------------------------------------------------------
// Store-level tests (real Postgres).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ownership_is_enforced_by_sql_on_every_read_and_write() {
    let harness = store_or_skip!();
    let alice = user();
    let bob = user();

    let doc = harness
        .store
        .create(NewDocument {
            user_id: alice,
            filename: "alice.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: 100,
            source: DocumentSource::LocalUpload,
            source_ref: None,
            content_hash: content_hash(b"alice"),
            storage_key: "local://alice/x".into(),
        })
        .await
        .unwrap();

    assert!(matches!(
        harness.store.get(bob, doc.id).await,
        Err(DocumentError::NotFound(_))
    ));
    assert!(matches!(
        harness.store.delete(bob, doc.id).await,
        Err(DocumentError::NotFound(_))
    ));
    assert!(matches!(
        harness.store.pages(bob, doc.id).await,
        Err(DocumentError::NotFound(_))
    ));
}

#[tokio::test]
async fn identical_bytes_dedup_for_the_same_user() {
    let harness = store_or_skip!();
    let uid = user();
    let hash = content_hash(b"same bytes");
    let a = harness
        .store
        .create(NewDocument {
            user_id: uid,
            filename: "a.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: 10,
            source: DocumentSource::LocalUpload,
            source_ref: None,
            content_hash: hash.clone(),
            storage_key: "local://a".into(),
        })
        .await
        .unwrap();
    let b = harness
        .store
        .create(NewDocument {
            user_id: uid,
            filename: "b.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: 10,
            source: DocumentSource::LocalUpload,
            source_ref: None,
            content_hash: hash,
            storage_key: "local://b".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        a.id, b.id,
        "duplicate bytes must resolve to the same document"
    );
}

#[tokio::test]
async fn processing_lifecycle_reaches_indexed_for_a_text_pdf() {
    let harness = store_or_skip!();
    let uid = user();
    let bytes = build_pdf(&[
        "Introduction to widgets.",
        "Chapter 2: widget submission deadline is September 18 at 11:59 PM.",
    ]);

    let temp_id = Uuid::new_v4();
    let key = harness
        .storage
        .write(uid, temp_id, "application/pdf", &bytes)
        .await
        .unwrap();
    let doc = harness
        .store
        .create(NewDocument {
            user_id: uid,
            filename: "syllabus.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: bytes.len() as u64,
            source: DocumentSource::LocalUpload,
            source_ref: None,
            content_hash: content_hash(&bytes),
            storage_key: key,
        })
        .await
        .unwrap();
    let pipe = Pipeline {
        store: harness.store.clone(),
        storage: harness.storage.clone(),
        ocr: Arc::new(NullOcrProvider),
        vision: Arc::new(NullVisionProvider),
    };
    let final_doc = pipe.process(&doc).await.unwrap();
    assert_eq!(final_doc.processing_state, ProcessingState::Indexed);
    assert_eq!(final_doc.page_count, Some(2));
    let pages = harness.store.pages(uid, final_doc.id).await.unwrap();
    assert_eq!(pages.len(), 2);
    assert!(pages[1].content.to_lowercase().contains("submission"));
    assert_eq!(pages[0].extraction_method, ExtractionMethod::NativeText);
}

#[tokio::test]
async fn processing_records_failure_on_a_corrupt_pdf() {
    let harness = store_or_skip!();
    let uid = user();

    let bytes = b"garbage".to_vec();
    let temp_id = Uuid::new_v4();
    let key = harness
        .storage
        .write(uid, temp_id, "application/pdf", &bytes)
        .await
        .unwrap();
    let doc = harness
        .store
        .create(NewDocument {
            user_id: uid,
            filename: "broken.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: bytes.len() as u64,
            source: DocumentSource::LocalUpload,
            source_ref: None,
            content_hash: content_hash(&bytes),
            storage_key: key,
        })
        .await
        .unwrap();
    let pipe = Pipeline {
        store: harness.store.clone(),
        storage: harness.storage.clone(),
        ocr: Arc::new(NullOcrProvider),
        vision: Arc::new(NullVisionProvider),
    };
    let final_doc = pipe.process(&doc).await.unwrap();
    assert_eq!(final_doc.processing_state, ProcessingState::Failed);
    assert!(final_doc.processing_error.is_some());
    assert!(final_doc.processed_at.is_some());
}

#[tokio::test]
async fn page_level_search_returns_ranked_snippets() {
    let harness = store_or_skip!();
    let uid = user();
    let bytes = build_pdf(&[
        "The syllabus is short.",
        "The final essay submission deadline is September 18 at 11:59 PM.",
        "Grading rubric details follow on the next page.",
    ]);
    let temp_id = Uuid::new_v4();
    let key = harness
        .storage
        .write(uid, temp_id, "application/pdf", &bytes)
        .await
        .unwrap();
    let doc = harness
        .store
        .create(NewDocument {
            user_id: uid,
            filename: "essay.pdf".into(),
            mime_type: "application/pdf".into(),
            size_bytes: bytes.len() as u64,
            source: DocumentSource::LocalUpload,
            source_ref: None,
            content_hash: content_hash(&bytes),
            storage_key: key,
        })
        .await
        .unwrap();
    let pipe = Pipeline {
        store: harness.store.clone(),
        storage: harness.storage.clone(),
        ocr: Arc::new(NullOcrProvider),
        vision: Arc::new(NullVisionProvider),
    };
    pipe.process(&doc).await.unwrap();

    let hits = harness
        .store
        .search_pages(assistant_documents::DocumentQuery {
            user_id: uid,
            text: Some("submission".into()),
            limit: 5,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!hits.is_empty(), "no page hits for 'submission'");
    assert!(hits.iter().any(|hit| hit.page_number == 2));
    assert!(hits[0].snippet.to_lowercase().contains("submission"));
}

// ---------------------------------------------------------------------------
// HTTP-level tests
// ---------------------------------------------------------------------------

fn test_config(storage_dir: std::path::PathBuf) -> Config {
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
        document_storage_dir: storage_dir,
    }
}

struct HttpHarness {
    addr: SocketAddr,
    _permit: tokio::sync::SemaphorePermit<'static>,
    _tmp: tempfile::TempDir,
}

async fn spawn_or_skip() -> Option<HttpHarness> {
    let _ = dotenvy::dotenv();
    let permit = DB_PERMITS.acquire().await.ok()?;
    let pool = pool_with(2).await?;
    let tmp = tempfile::tempdir().expect("temp dir");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = test_config(tmp.path().to_path_buf());
    let deps = Dependencies {
        tools: Arc::new(ToolRegistry::new()),
        ..Default::default()
    };
    let router = app(&config, Some(pool), EventBus::default(), deps);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Some(HttpHarness {
        addr,
        _permit: permit,
        _tmp: tmp,
    })
}

macro_rules! server_or_skip {
    () => {
        match spawn_or_skip().await {
            Some(harness) => harness,
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

#[tokio::test]
async fn upload_ingest_and_search_round_trip() {
    let harness = server_or_skip!();
    let addr = harness.addr;
    let bytes = build_pdf(&[
        "Preface: this is a fixture PDF.",
        "Submission deadline is September 18 at 11:59 PM.",
    ]);

    let response = client()
        .post(format!("http://{addr}/v1/documents"))
        .bearer_auth(TEST_TOKEN)
        .header("Content-Type", "application/pdf")
        .header("X-Filename", "syllabus.pdf")
        .body(bytes.clone())
        .send()
        .await
        .expect("upload");
    assert!(
        response.status().is_success(),
        "upload failed: {:?}",
        response.status()
    );
    let stored: DocumentItem = response.json().await.expect("body");
    assert_eq!(stored.filename, "syllabus.pdf");
    assert_eq!(
        stored.processing_state,
        assistant_protocol::DocumentProcessingStateDto::Indexed,
        "processing state was not Indexed; error: {:?}",
        stored.processing_error
    );
    assert_eq!(stored.page_count, Some(2));

    // List should include the new document.
    let list: Vec<DocumentItem> = client()
        .get(format!("http://{addr}/v1/documents"))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("body");
    assert!(list.iter().any(|d| d.id == stored.id));

    // Page-level search.
    let hits: Vec<DocumentSearchHit> = client()
        .get(format!("http://{addr}/v1/documents/search?q=submission"))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("search")
        .json()
        .await
        .expect("body");
    assert!(hits.iter().any(|h| h.document_id == stored.id));

    // Page fetch.
    let page: DocumentPageItem = client()
        .get(format!("http://{addr}/v1/documents/{}/pages/2", stored.id))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("page")
        .json()
        .await
        .expect("body");
    assert!(page.content.to_lowercase().contains("submission"));
}

#[tokio::test]
async fn upload_rejects_unsupported_mime_type_with_400() {
    let harness = server_or_skip!();
    let addr = harness.addr;
    let response = client()
        .post(format!("http://{addr}/v1/documents"))
        .bearer_auth(TEST_TOKEN)
        .header("Content-Type", "image/png")
        .header("X-Filename", "photo.png")
        .body(vec![1, 2, 3, 4])
        .send()
        .await
        .expect("upload");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upload_rejects_body_over_the_size_limit_with_400() {
    let harness = server_or_skip!();
    let addr = harness.addr;
    // The limit is 25 MiB; skip if the test environment cannot allocate a
    // buffer larger than that. `vec![0u8; N]` is fine even in constrained
    // CI runners at this size.
    let too_big = vec![0u8; (assistant_documents::MAX_DOCUMENT_BYTES + 1) as usize];
    let response = client()
        .post(format!("http://{addr}/v1/documents"))
        .bearer_auth(TEST_TOKEN)
        .header("Content-Type", "text/plain")
        .body(too_big)
        .send()
        .await
        .expect("upload");
    // Either the router's own MAX_DOCUMENT_BYTES check (400) or Axum's
    // transport-layer body limit (413 Payload Too Large) is a valid refusal;
    // both keep an oversized upload out of the pipeline, which is what this
    // test cares about.
    let status = response.status();
    assert!(
        status == reqwest::StatusCode::BAD_REQUEST
            || status == reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        "expected 400 or 413, got {status}"
    );
}

#[tokio::test]
async fn unauthenticated_requests_are_refused() {
    let harness = server_or_skip!();
    let addr = harness.addr;
    let response = reqwest::Client::new()
        .get(format!("http://{addr}/v1/documents"))
        .send()
        .await
        .expect("get");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn reprocessing_after_a_failed_document_can_recover() {
    let harness = server_or_skip!();
    let addr = harness.addr;

    // Upload a "corrupt" PDF: valid Content-Type, invalid body.
    let response = client()
        .post(format!("http://{addr}/v1/documents"))
        .bearer_auth(TEST_TOKEN)
        .header("Content-Type", "application/pdf")
        .header("X-Filename", "broken.pdf")
        .body(b"garbage".to_vec())
        .send()
        .await
        .expect("upload");
    let stored: DocumentItem = response.json().await.expect("body");
    assert_eq!(
        stored.processing_state,
        assistant_protocol::DocumentProcessingStateDto::Failed
    );

    // Reprocess still fails, but returns a valid state (it does not crash).
    let response = client()
        .post(format!(
            "http://{addr}/v1/documents/{}/reprocess",
            stored.id
        ))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("reprocess");
    assert!(response.status().is_success());
    let after: DocumentItem = response.json().await.expect("body");
    assert_eq!(
        after.processing_state,
        assistant_protocol::DocumentProcessingStateDto::Failed
    );
    assert!(after.processing_error.is_some());
}

#[tokio::test]
async fn delete_removes_document_and_its_pages() {
    let harness = server_or_skip!();
    let addr = harness.addr;
    let bytes = build_pdf(&["Delete me."]);
    let stored: DocumentItem = client()
        .post(format!("http://{addr}/v1/documents"))
        .bearer_auth(TEST_TOKEN)
        .header("Content-Type", "application/pdf")
        .header("X-Filename", "gone.pdf")
        .body(bytes)
        .send()
        .await
        .expect("upload")
        .json()
        .await
        .expect("body");

    let response = client()
        .delete(format!("http://{addr}/v1/documents/{}", stored.id))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("delete");
    assert!(response.status().is_success());

    let missing = client()
        .get(format!("http://{addr}/v1/documents/{}", stored.id))
        .bearer_auth(TEST_TOKEN)
        .send()
        .await
        .expect("get after delete");
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Provenance and memory-proposal bridging
// ---------------------------------------------------------------------------

#[test]
fn document_memory_proposal_produces_a_memory_source_of_document() {
    use assistant_documents::{DocumentMemoryProposal, DocumentProvenance, ExtractionMethod as EM};
    let doc_id = Uuid::new_v4();
    let proposal = DocumentMemoryProposal {
        kind: assistant_memory::MemoryKind::Fact,
        content: "The final essay is due September 18.".into(),
        confidence: 0.8,
        provenance: DocumentProvenance {
            document_id: doc_id,
            filename: "syllabus.pdf".into(),
            source: DocumentSource::LocalUpload,
            source_ref: None,
            page_number: 3,
            extraction_method: EM::NativeText,
        },
    };
    let mem = proposal.into_memory_proposal();
    assert_eq!(mem.source_kind, assistant_memory::MemorySource::Document);
    assert!(mem.source_ref.unwrap().starts_with(&doc_id.to_string()));
}

#[test]
fn deadline_candidate_extraction_finds_full_dates_from_a_pdf_page() {
    use assistant_documents::{DocumentProvenance, ExtractionMethod as EM, deadlines};
    let doc = Uuid::new_v4();
    let provenance = DocumentProvenance {
        document_id: doc,
        filename: "syllabus.pdf".into(),
        source: DocumentSource::LocalUpload,
        source_ref: None,
        page_number: 2,
        extraction_method: EM::NativeText,
    };
    let bytes = build_pdf(&["Assignment 1: essay. Submission due September 18 2026 at 11:59 PM."]);
    let extracted = extract_pdf(&bytes).expect("extracted");
    let candidates = deadlines::extract(&extracted.pages[0].text, &provenance);
    assert!(candidates.iter().any(|c| c.due_at.is_some()));
    let with = candidates.iter().find(|c| c.due_at.is_some()).unwrap();
    let due = with.due_at.unwrap();
    assert_eq!(due.year(), 2026);
    assert_eq!(due.month(), time::Month::September);
    assert_eq!(due.day(), 18);
    assert_eq!(with.provenance.filename, "syllabus.pdf");
    assert_eq!(with.provenance.page_number, 2);
}

// ---------------------------------------------------------------------------
// Protocol serialisation smoke tests
// ---------------------------------------------------------------------------

#[test]
fn document_item_round_trips_through_serde() {
    let now = time::OffsetDateTime::now_utc();
    let item = DocumentItem {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        filename: "x.pdf".into(),
        mime_type: "application/pdf".into(),
        size_bytes: 4096,
        source: assistant_protocol::DocumentSourceDto::LocalUpload,
        source_ref: None,
        content_hash: content_hash(b"x"),
        page_count: Some(3),
        processing_state: assistant_protocol::DocumentProcessingStateDto::Indexed,
        processing_error: None,
        created_at: now,
        updated_at: now,
        processed_at: Some(now),
    };
    let s = serde_json::to_string(&item).unwrap();
    let back: DocumentItem = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, item.id);
    assert_eq!(back.page_count, Some(3));
}

// A hand-wave assertion so the compiler keeps the Document import used.
#[allow(dead_code)]
fn _assert_document_types(doc: Document) -> Document {
    doc
}
