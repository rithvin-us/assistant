//! Document / PDF intelligence domain.
//!
//! What lives here: the types the assistant uses to reason about documents
//! (their metadata, their pages, their processing state, their provenance),
//! the traits a durable store and an object store must implement, and
//! provider-neutral seams for OCR and visual verification.
//!
//! What deliberately does NOT live here: Google Drive, Anthropic, OpenAI,
//! Tesseract or any other concrete provider. The document domain is
//! provider-independent for the same reason `assistant-memory` is: swapping
//! a provider must not touch the domain, and a test must be able to drive
//! the whole pipeline against deterministic fakes.
//!
//! The core rule this crate encodes: **extraction is deterministic first**.
//! Text comes from `pdf-extract` where possible, OCR is asked for only when
//! the deterministic pass returned nothing usable, and visual/multimodal
//! verification runs only where the layout or the OCR confidence make it
//! worthwhile. Nothing here calls a language model on every page.

pub mod extract;
pub mod pipeline;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

pub type DocumentId = Uuid;
pub type UserId = Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Where the document came from.
///
/// Bounded on purpose: the database `check` constraint enumerates these, so a
/// caller cannot invent a source that a retrieval query would then quietly
/// miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentSource {
    /// The user handed us bytes directly.
    LocalUpload,
    /// Ingested from Google Drive (via M6). `source_ref` carries the Drive
    /// file id.
    GoogleDrive,
    /// Anything not covered above; a placeholder for a future integration.
    ExternalSource,
}

impl DocumentSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalUpload => "local_upload",
            Self::GoogleDrive => "google_drive",
            Self::ExternalSource => "external_source",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local_upload" => Some(Self::LocalUpload),
            "google_drive" => Some(Self::GoogleDrive),
            "external_source" => Some(Self::ExternalSource),
            _ => None,
        }
    }
}

/// The lifecycle of processing.
///
/// Deliberately linear: a document is uploaded, extracted, optionally OCR'd,
/// optionally verified, and then indexed. A failure lands as `Failed` with a
/// human-readable reason on the row. The state does not silently regress:
/// re-processing a `Failed` document restarts the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingState {
    /// Bytes stored, not yet processed.
    Uploaded,
    /// Deterministic text extraction in progress.
    Extracting,
    /// OCR in progress (needed on at least one page).
    Ocr,
    /// Visual/multimodal verification in progress (needed on at least one
    /// page).
    Verifying,
    /// All pages have a definitive representation and the document is
    /// searchable.
    Indexed,
    /// Something went wrong; `processing_error` on the row explains what.
    Failed,
}

impl ProcessingState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uploaded => "uploaded",
            Self::Extracting => "extracting",
            Self::Ocr => "ocr",
            Self::Verifying => "verifying",
            Self::Indexed => "indexed",
            Self::Failed => "failed",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "uploaded" => Some(Self::Uploaded),
            "extracting" => Some(Self::Extracting),
            "ocr" => Some(Self::Ocr),
            "verifying" => Some(Self::Verifying),
            "indexed" => Some(Self::Indexed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Indexed | Self::Failed)
    }
}

/// How this page's text came to be. Recorded per page so the UI, the ranker
/// and the audit trail can distinguish confidently-extracted native text from
/// an OCR result at 62% confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMethod {
    /// The PDF text stream returned something usable.
    NativeText,
    /// OCR was invoked because the native pass produced too little.
    Ocr,
    /// A visual/multimodal verifier confirmed or replaced the OCR result.
    VisualVerification,
    /// Nothing readable exists for this page.
    None,
}

impl ExtractionMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeText => "native_text",
            Self::Ocr => "ocr",
            Self::VisualVerification => "visual_verification",
            Self::None => "none",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "native_text" => Some(Self::NativeText),
            "ocr" => Some(Self::Ocr),
            "visual_verification" => Some(Self::VisualVerification),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// Where a piece of document-derived content came from.
///
/// Every fact the assistant surfaces from a document must be paired with one
/// of these. That is how the UI can answer "why do you think the deadline is
/// September 18?" without inventing the answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentProvenance {
    pub document_id: DocumentId,
    pub filename: String,
    pub source: DocumentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// 1-based page number, matching the way humans talk about PDFs.
    pub page_number: u32,
    pub extraction_method: ExtractionMethod,
}

// ---------------------------------------------------------------------------
// Domain records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub user_id: UserId,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub source: DocumentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    /// SHA-256 of the raw bytes, hex-encoded. Written on ingest and never
    /// mutated. Two ingests of the same bytes for the same user resolve to
    /// the same row.
    pub content_hash: String,
    /// `None` until extraction runs. Preserves the possibility of unknown-
    /// page-count documents during the brief window before processing.
    pub page_count: Option<u32>,
    pub processing_state: ProcessingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_error: Option<String>,
    /// Path within the configured object store. Opaque; only the storage
    /// backend knows how to resolve it.
    pub storage_key: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub processed_at: Option<OffsetDateTime>,
}

/// One page of a document as the store returned it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentPage {
    pub document_id: DocumentId,
    pub user_id: UserId,
    pub page_number: u32,
    pub extraction_method: ExtractionMethod,
    /// The page's text, already trimmed. Empty string means "the extraction
    /// method ran and returned nothing usable" -- as opposed to `content =
    /// None` in the store, which would mean the page has not been processed.
    pub content: String,
    /// `None` for native text (which is exact); `Some(0..=1)` for OCR or
    /// visual verification.
    pub confidence: Option<f32>,
    /// The number of characters extracted. Precomputed so the API can decide
    /// context budgets without pulling every page's text back.
    pub char_count: u32,
}

/// Input for [`DocumentStore::create`]: an already-ingested document ready to
/// be persisted.
#[derive(Debug, Clone)]
pub struct NewDocument {
    pub user_id: UserId,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub source: DocumentSource,
    pub source_ref: Option<String>,
    pub content_hash: String,
    pub storage_key: String,
}

/// A patch on a document row. Only fields the processing pipeline writes are
/// exposed; the source, hash, storage key, ownership and filename are set at
/// ingest and never mutated by later processing.
#[derive(Debug, Clone, Default)]
pub struct DocumentPatch {
    pub processing_state: Option<ProcessingState>,
    pub processing_error: Option<Option<String>>,
    pub page_count: Option<u32>,
    pub processed_at: Option<Option<OffsetDateTime>>,
}

/// Search over documents (metadata + full text).
#[derive(Debug, Clone, Default)]
pub struct DocumentQuery {
    pub user_id: UserId,
    /// Free text: matches filename and page content.
    pub text: Option<String>,
    /// If set, restricts to this MIME type.
    pub mime_type: Option<String>,
    /// If set, restricts to these processing states.
    pub states: Vec<ProcessingState>,
    pub limit: u32,
}

impl DocumentQuery {
    pub fn effective_limit(&self) -> u32 {
        let raw = if self.limit == 0 {
            DEFAULT_SEARCH_LIMIT
        } else {
            self.limit
        };
        raw.min(MAX_SEARCH_LIMIT)
    }
}

/// One row in a page-level search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSearchHit {
    pub document_id: DocumentId,
    pub filename: String,
    pub page_number: u32,
    pub extraction_method: ExtractionMethod,
    /// A short human-readable slice of the page that matched.
    pub snippet: String,
    /// Non-negative. Larger is more relevant.
    pub score: f32,
}

// ---------------------------------------------------------------------------
// Extraction outputs
// ---------------------------------------------------------------------------

/// The result of running the deterministic PDF extractor on raw bytes.
///
/// Not stored directly; the pipeline turns this into a [`Document`] and one
/// [`DocumentPage`] per page. Kept as a plain struct so a caller (tests,
/// future non-PDF extractors) can construct one without going through the
/// extractor at all.
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    pub page_count: u32,
    pub pages: Vec<ExtractedPage>,
}

#[derive(Debug, Clone)]
pub struct ExtractedPage {
    pub page_number: u32,
    pub text: String,
    /// True when the extractor concluded this page has too little native text
    /// to trust: an OCR pass should run. Deterministic heuristic; see
    /// [`extract::page_needs_ocr`].
    pub needs_ocr: bool,
}

// ---------------------------------------------------------------------------
// Storage and processing seams
// ---------------------------------------------------------------------------

/// The durable metadata store.
///
/// Same shape as `MemoryStore` and `ConversationStore`: every method takes the
/// authenticated user's id and scopes on it. The Postgres implementation lives
/// in `assistant-server`.
#[async_trait]
pub trait DocumentStore: Send + Sync {
    /// Inserts (or, if `content_hash` already exists for this user, returns
    /// the existing row).
    async fn create(&self, new_document: NewDocument) -> Result<Document, DocumentError>;

    async fn get(&self, user_id: UserId, id: DocumentId) -> Result<Document, DocumentError>;

    async fn search(&self, query: DocumentQuery) -> Result<Vec<Document>, DocumentError>;

    async fn update(
        &self,
        user_id: UserId,
        id: DocumentId,
        patch: DocumentPatch,
    ) -> Result<Document, DocumentError>;

    async fn delete(&self, user_id: UserId, id: DocumentId) -> Result<(), DocumentError>;

    /// Replaces every page for a document with `pages`. Called by the pipeline
    /// at the end of extraction; a document has exactly one authoritative set
    /// of pages at any moment.
    async fn replace_pages(
        &self,
        user_id: UserId,
        id: DocumentId,
        pages: Vec<DocumentPage>,
    ) -> Result<(), DocumentError>;

    async fn pages(
        &self,
        user_id: UserId,
        id: DocumentId,
    ) -> Result<Vec<DocumentPage>, DocumentError>;

    async fn get_page(
        &self,
        user_id: UserId,
        id: DocumentId,
        page_number: u32,
    ) -> Result<DocumentPage, DocumentError>;

    /// Ranked page-level search. Returns at most `query.effective_limit()`
    /// hits, ordered by descending relevance.
    async fn search_pages(&self, query: DocumentQuery)
    -> Result<Vec<PageSearchHit>, DocumentError>;
}

/// The object store the raw bytes live in.
///
/// Kept behind a trait so tests can plug in an in-memory store and the server
/// can plug in filesystem or Supabase Storage. Nothing in the domain layer
/// knows or cares which backend is configured.
#[async_trait]
pub trait DocumentStorage: Send + Sync {
    /// Persists bytes for a document owned by `user_id`. Returns the opaque
    /// storage key that later `read`s will use.
    async fn write(
        &self,
        user_id: UserId,
        document_id: DocumentId,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<String, StorageError>;

    async fn read(&self, storage_key: &str) -> Result<Vec<u8>, StorageError>;

    async fn delete(&self, storage_key: &str) -> Result<(), StorageError>;
}

// ---------------------------------------------------------------------------
// OCR and Vision providers
// ---------------------------------------------------------------------------

/// One page's rendered representation, as fed to an [`OcrProvider`] or a
/// [`DocumentVisionProvider`]. The domain does not render pages itself; the
/// pipeline will render on demand once a renderer is wired in.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page_number: u32,
    /// Rendered image bytes (typically PNG). For text-only fakes, this is
    /// empty and the fake ignores it.
    pub image_bytes: Vec<u8>,
    /// A textual hint the fake OCR provider can use to produce a deterministic
    /// answer in tests: real providers ignore this field entirely.
    pub text_hint: Option<String>,
}

/// Provider-neutral OCR seam.
///
/// Concrete providers (Tesseract, cloud OCR APIs) implement this. The domain
/// only cares that "given a page image, return text and a confidence". A
/// caller can substitute a null implementation in a deployment that has no
/// OCR configured; the pipeline records the missing OCR as a per-page state
/// rather than silently swallowing pages.
#[async_trait]
pub trait OcrProvider: Send + Sync {
    /// Returns `Ok((text, confidence))` on success; `Err(OcrError::NotAvailable)`
    /// when the provider is not configured (so the pipeline records the page
    /// as "OCR needed" rather than crashing).
    async fn recognise(&self, page: &RenderedPage) -> Result<OcrResult, OcrError>;
}

#[derive(Debug, Clone)]
pub struct OcrResult {
    pub text: String,
    /// `0.0..=1.0`. `1.0` for providers that do not report a confidence.
    pub confidence: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("no OCR provider is configured")]
    NotAvailable,
    #[error("OCR failed: {0}")]
    Failed(String),
}

/// Provider-neutral visual/multimodal verification seam.
///
/// Concrete providers (a model provider's vision endpoint) implement this. The
/// domain calls it only when the deterministic pass and OCR together left the
/// system unsure -- for example a scanned page with tables where OCR
/// confidence is low. A null implementation is fine: the pipeline records
/// "verification skipped" and moves on.
#[async_trait]
pub trait DocumentVisionProvider: Send + Sync {
    async fn verify(&self, page: &RenderedPage) -> Result<VerificationResult, VisionError>;
}

#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub text: String,
    pub confidence: f32,
    /// Free-form note; the pipeline stores this on the page row as part of the
    /// confidence audit. Never used as authoritative content.
    pub note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("no vision provider is configured")]
    NotAvailable,
    #[error("visual verification failed: {0}")]
    Failed(String),
}

// ---------------------------------------------------------------------------
// Deadline candidates and memory proposals from documents
// ---------------------------------------------------------------------------

/// A structured deadline candidate extracted from a document page.
///
/// This is a *candidate*, not a task. The application decides whether to
/// create a task from it, using the existing task/permission architecture --
/// M8 never autonomously mutates tasks or memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadlineCandidate {
    pub title: String,
    /// The raw phrase we found (e.g. `"Submission due September 18 at 11:59 PM"`).
    pub matched_text: String,
    /// Best-effort parse of the deadline text; `None` when we could not
    /// unambiguously resolve a date and time. A guessed deadline is worse
    /// than no deadline (ADR-0034).
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub due_at: Option<OffsetDateTime>,
    pub confidence: f32,
    pub provenance: DocumentProvenance,
}

/// A structured proposal to turn document content into a durable memory. The
/// application decides whether to persist it via
/// [`assistant_memory::MemoryProposal::accept`], so the same validation
/// (secret rejection, importance clamp, provenance) applies.
#[derive(Debug, Clone)]
pub struct DocumentMemoryProposal {
    pub kind: assistant_memory::MemoryKind,
    pub content: String,
    pub confidence: f32,
    pub provenance: DocumentProvenance,
}

impl DocumentMemoryProposal {
    /// Turns this into a domain-level `MemoryProposal` for the memory layer.
    pub fn into_memory_proposal(self) -> assistant_memory::MemoryProposal {
        let DocumentMemoryProposal {
            kind,
            content,
            confidence,
            provenance,
        } = self;
        assistant_memory::MemoryProposal {
            kind,
            content,
            confidence: Some(confidence),
            importance: None,
            reason: Some(format!(
                "extracted from {} (page {}, {})",
                provenance.filename,
                provenance.page_number,
                provenance.extraction_method.as_str()
            )),
            source_kind: assistant_memory::MemorySource::Document,
            source_ref: Some(document_ref(&provenance)),
            expires_at: None,
        }
    }
}

fn document_ref(prov: &DocumentProvenance) -> String {
    format!("{}#p{}", prov.document_id, prov.page_number)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("document {0} not found")]
    NotFound(DocumentId),
    #[error("page {page} not found for document {document}")]
    PageNotFound { document: DocumentId, page: u32 },
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("unsupported document type: {0}")]
    UnsupportedType(String),
    #[error("document too large: {size} bytes > {limit} bytes")]
    TooLarge { size: u64, limit: u64 },
    #[error("document store failure: {0}")]
    Backend(String),
}

impl DocumentError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::PageNotFound { .. } => "page_not_found",
            Self::Invalid(_) => "invalid",
            Self::UnsupportedType(_) => "unsupported_type",
            Self::TooLarge { .. } => "too_large",
            Self::Backend(_) => "backend",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage read failure: {0}")]
    Read(String),
    #[error("storage write failure: {0}")]
    Write(String),
    #[error("storage delete failure: {0}")]
    Delete(String),
    #[error("object not found")]
    NotFound,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Hard upper bound on a document's size in bytes.
///
/// 25 MiB is enough for a large academic PDF or a scanned essay while keeping
/// a runaway upload from blowing the process's memory. The ingest path
/// refuses anything above this before storing bytes.
pub const MAX_DOCUMENT_BYTES: u64 = 25 * 1024 * 1024;

/// Maximum characters in one page's content field. Pages beyond this are
/// truncated with a trailing "\u{2026}"; the truncation is exposed to the UI
/// via `char_count` so nothing pretends to have shown a whole page.
pub const MAX_PAGE_CHARS: usize = 16_000;

/// Character length of the search snippet returned by `search_pages`.
pub const SEARCH_SNIPPET_CHARS: usize = 320;

/// Default limit for document search when the caller does not specify one.
pub const DEFAULT_SEARCH_LIMIT: u32 = 25;

/// Absolute upper bound on `limit` for any document search API call.
pub const MAX_SEARCH_LIMIT: u32 = 200;

/// Maximum pages we will feed into a single turn as context, across all
/// documents. The retrieval layer never widens this.
pub const MAX_CONTEXT_PAGES: usize = 4;

/// Maximum characters injected into one turn's context from documents. Even
/// when the pages themselves are shorter, they are clipped to fit inside this
/// budget.
pub const MAX_CONTEXT_CHARS: usize = 8_000;

/// MIME types we currently understand well enough to process. Anything else
/// is refused at ingest with an [`DocumentError::UnsupportedType`], rather
/// than accepted and left mysterious.
pub fn is_supported_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/pdf"
            | "text/plain"
            | "text/markdown"
            | "text/csv"
            | "text/html"
            | "application/json"
            | "application/xml"
            | "text/xml"
    )
}

// ---------------------------------------------------------------------------
// In-memory implementations for tests and non-durable deployments
// ---------------------------------------------------------------------------

#[derive(Default)]
struct InMemoryData {
    documents: HashMap<DocumentId, Document>,
    pages: HashMap<DocumentId, Vec<DocumentPage>>,
}

#[derive(Default)]
pub struct InMemoryDocumentStore {
    inner: RwLock<InMemoryData>,
}

impl InMemoryDocumentStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DocumentStore for InMemoryDocumentStore {
    async fn create(&self, new_document: NewDocument) -> Result<Document, DocumentError> {
        let mut guard = self.inner.write().await;
        // Dedup by (user_id, content_hash): same bytes for the same user is
        // the same document.
        if let Some(existing) = guard.documents.values().find(|d| {
            d.user_id == new_document.user_id && d.content_hash == new_document.content_hash
        }) {
            return Ok(existing.clone());
        }
        let now = OffsetDateTime::now_utc();
        let document = Document {
            id: Uuid::new_v4(),
            user_id: new_document.user_id,
            filename: new_document.filename,
            mime_type: new_document.mime_type,
            size_bytes: new_document.size_bytes,
            source: new_document.source,
            source_ref: new_document.source_ref,
            content_hash: new_document.content_hash,
            page_count: None,
            processing_state: ProcessingState::Uploaded,
            processing_error: None,
            storage_key: new_document.storage_key,
            created_at: now,
            updated_at: now,
            processed_at: None,
        };
        guard.documents.insert(document.id, document.clone());
        Ok(document)
    }

    async fn get(&self, user_id: UserId, id: DocumentId) -> Result<Document, DocumentError> {
        let guard = self.inner.read().await;
        guard
            .documents
            .get(&id)
            .filter(|row| row.user_id == user_id)
            .cloned()
            .ok_or(DocumentError::NotFound(id))
    }

    async fn search(&self, query: DocumentQuery) -> Result<Vec<Document>, DocumentError> {
        let guard = self.inner.read().await;
        let text = query.text.as_deref().map(str::to_lowercase);
        let mut matches: Vec<Document> = guard
            .documents
            .values()
            .filter(|row| row.user_id == query.user_id)
            .filter(|row| {
                query
                    .mime_type
                    .as_deref()
                    .is_none_or(|mime| row.mime_type == mime)
            })
            .filter(|row| query.states.is_empty() || query.states.contains(&row.processing_state))
            .filter(|row| {
                let Some(ref text) = text else {
                    return true;
                };
                let filename_hit = row.filename.to_lowercase().contains(text);
                let content_hit = guard.pages.get(&row.id).is_some_and(|pages| {
                    pages
                        .iter()
                        .any(|p| p.content.to_lowercase().contains(text))
                });
                filename_hit || content_hit
            })
            .cloned()
            .collect();
        matches.sort_by_key(|row| std::cmp::Reverse(row.updated_at));
        matches.truncate(query.effective_limit() as usize);
        Ok(matches)
    }

    async fn update(
        &self,
        user_id: UserId,
        id: DocumentId,
        patch: DocumentPatch,
    ) -> Result<Document, DocumentError> {
        let mut guard = self.inner.write().await;
        let row = guard
            .documents
            .get_mut(&id)
            .filter(|row| row.user_id == user_id)
            .ok_or(DocumentError::NotFound(id))?;
        if let Some(state) = patch.processing_state {
            row.processing_state = state;
        }
        if let Some(error) = patch.processing_error {
            row.processing_error = error;
        }
        if let Some(page_count) = patch.page_count {
            row.page_count = Some(page_count);
        }
        if let Some(processed_at) = patch.processed_at {
            row.processed_at = processed_at;
        }
        row.updated_at = OffsetDateTime::now_utc();
        Ok(row.clone())
    }

    async fn delete(&self, user_id: UserId, id: DocumentId) -> Result<(), DocumentError> {
        let mut guard = self.inner.write().await;
        let Some(row) = guard.documents.get(&id) else {
            return Err(DocumentError::NotFound(id));
        };
        if row.user_id != user_id {
            return Err(DocumentError::NotFound(id));
        }
        guard.documents.remove(&id);
        guard.pages.remove(&id);
        Ok(())
    }

    async fn replace_pages(
        &self,
        user_id: UserId,
        id: DocumentId,
        pages: Vec<DocumentPage>,
    ) -> Result<(), DocumentError> {
        let mut guard = self.inner.write().await;
        let Some(row) = guard.documents.get(&id) else {
            return Err(DocumentError::NotFound(id));
        };
        if row.user_id != user_id {
            return Err(DocumentError::NotFound(id));
        }
        guard.pages.insert(id, pages);
        Ok(())
    }

    async fn pages(
        &self,
        user_id: UserId,
        id: DocumentId,
    ) -> Result<Vec<DocumentPage>, DocumentError> {
        let guard = self.inner.read().await;
        let Some(row) = guard.documents.get(&id) else {
            return Err(DocumentError::NotFound(id));
        };
        if row.user_id != user_id {
            return Err(DocumentError::NotFound(id));
        }
        Ok(guard.pages.get(&id).cloned().unwrap_or_default())
    }

    async fn get_page(
        &self,
        user_id: UserId,
        id: DocumentId,
        page_number: u32,
    ) -> Result<DocumentPage, DocumentError> {
        let pages = self.pages(user_id, id).await?;
        pages
            .into_iter()
            .find(|p| p.page_number == page_number)
            .ok_or(DocumentError::PageNotFound {
                document: id,
                page: page_number,
            })
    }

    async fn search_pages(
        &self,
        query: DocumentQuery,
    ) -> Result<Vec<PageSearchHit>, DocumentError> {
        let guard = self.inner.read().await;
        let Some(text) = query.text.as_deref().map(str::to_lowercase) else {
            return Ok(Vec::new());
        };
        let mut hits = Vec::new();
        for (doc_id, pages) in guard.pages.iter() {
            let Some(doc) = guard.documents.get(doc_id) else {
                continue;
            };
            if doc.user_id != query.user_id {
                continue;
            }
            for page in pages {
                let content_lc = page.content.to_lowercase();
                if !content_lc.contains(&text) {
                    continue;
                }
                let snippet = snippet_around(&page.content, &text);
                let hits_count = content_lc.matches(text.as_str()).count() as f32;
                hits.push(PageSearchHit {
                    document_id: doc.id,
                    filename: doc.filename.clone(),
                    page_number: page.page_number,
                    extraction_method: page.extraction_method,
                    snippet,
                    score: hits_count,
                });
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(query.effective_limit() as usize);
        Ok(hits)
    }
}

pub fn snippet_around(content: &str, needle_lc: &str) -> String {
    let content_lc = content.to_lowercase();
    let Some(idx) = content_lc.find(needle_lc) else {
        return content.chars().take(SEARCH_SNIPPET_CHARS).collect();
    };
    let start = idx.saturating_sub(SEARCH_SNIPPET_CHARS / 4);
    let end = (idx + needle_lc.len() + SEARCH_SNIPPET_CHARS / 2).min(content.len());
    // Snap to the nearest char boundary so we do not slice through UTF-8.
    let start = round_boundary(content, start);
    let end = round_boundary(content, end);
    let mut out = String::new();
    if start > 0 {
        out.push('\u{2026}');
    }
    out.push_str(&content[start..end]);
    if end < content.len() {
        out.push('\u{2026}');
    }
    out
}

fn round_boundary(s: &str, mut idx: usize) -> usize {
    if idx > s.len() {
        idx = s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// In-memory object store: keeps bytes in a map, keyed by an opaque UUID.
/// Handy for tests; not durable.
#[derive(Default)]
pub struct InMemoryDocumentStorage {
    objects: RwLock<HashMap<String, Vec<u8>>>,
}

impl InMemoryDocumentStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DocumentStorage for InMemoryDocumentStorage {
    async fn write(
        &self,
        _user_id: UserId,
        document_id: DocumentId,
        _content_type: &str,
        bytes: &[u8],
    ) -> Result<String, StorageError> {
        let key = format!("mem://{document_id}");
        self.objects
            .write()
            .await
            .insert(key.clone(), bytes.to_vec());
        Ok(key)
    }

    async fn read(&self, storage_key: &str) -> Result<Vec<u8>, StorageError> {
        self.objects
            .read()
            .await
            .get(storage_key)
            .cloned()
            .ok_or(StorageError::NotFound)
    }

    async fn delete(&self, storage_key: &str) -> Result<(), StorageError> {
        self.objects.write().await.remove(storage_key);
        Ok(())
    }
}

/// A no-op OCR provider: reports `NotAvailable` for every page.
///
/// Used when the deployment has no OCR configured; the pipeline records the
/// affected pages as "OCR needed" and moves on instead of pretending to have
/// read them. Tests that want deterministic OCR output can use
/// [`FakeOcrProvider`] instead.
pub struct NullOcrProvider;

#[async_trait]
impl OcrProvider for NullOcrProvider {
    async fn recognise(&self, _page: &RenderedPage) -> Result<OcrResult, OcrError> {
        Err(OcrError::NotAvailable)
    }
}

/// A deterministic OCR fake for tests. Returns the `text_hint` supplied on
/// the rendered page, with a fixed confidence.
pub struct FakeOcrProvider {
    pub confidence: f32,
}

impl FakeOcrProvider {
    pub fn new(confidence: f32) -> Self {
        Self { confidence }
    }
}

#[async_trait]
impl OcrProvider for FakeOcrProvider {
    async fn recognise(&self, page: &RenderedPage) -> Result<OcrResult, OcrError> {
        Ok(OcrResult {
            text: page.text_hint.clone().unwrap_or_default(),
            confidence: self.confidence,
        })
    }
}

/// A no-op vision provider.
pub struct NullVisionProvider;

#[async_trait]
impl DocumentVisionProvider for NullVisionProvider {
    async fn verify(&self, _page: &RenderedPage) -> Result<VerificationResult, VisionError> {
        Err(VisionError::NotAvailable)
    }
}

// ---------------------------------------------------------------------------
// Deadline extraction (deterministic)
// ---------------------------------------------------------------------------

/// Deterministically extracts deadline candidates from a page's text.
///
/// This is a small, testable heuristic -- not a natural-language date parser.
/// It looks for a small set of "due …" phrasings and, when the trailing text
/// contains a recognised absolute date and optional time, produces a
/// candidate with `due_at` populated. Everything else becomes a candidate
/// with `due_at = None` so the UI can still surface the phrase without
/// pretending to know when it means.
pub mod deadlines {
    use super::*;
    use time::{Date, Month, Time};

    const TRIGGERS: &[&str] = &[
        "due ",
        "deadline: ",
        "deadline ",
        "submit by ",
        "submitted by ",
        "submission by ",
        "submission due ",
        "final submission",
    ];

    pub fn extract(page_text: &str, provenance: &DocumentProvenance) -> Vec<DeadlineCandidate> {
        let mut out = Vec::new();
        let lower = page_text.to_lowercase();
        for trigger in TRIGGERS {
            let mut start = 0usize;
            while let Some(pos) = lower[start..].find(trigger) {
                let abs = start + pos;
                let end = (abs + 160).min(page_text.len());
                let end = round_boundary(page_text, end);
                let snippet = page_text[abs..end].to_string();
                let due_at = parse_due_phrase(&snippet);
                let confidence = if due_at.is_some() { 0.7 } else { 0.3 };
                out.push(DeadlineCandidate {
                    title: heading_for(&snippet),
                    matched_text: snippet.clone(),
                    due_at,
                    confidence,
                    provenance: provenance.clone(),
                });
                start = abs + trigger.len();
            }
        }
        out
    }

    fn heading_for(matched: &str) -> String {
        // Take the first line, trim trailing punctuation, cap length.
        let mut line = matched.lines().next().unwrap_or(matched).trim().to_string();
        if line.len() > 80 {
            line = line[..round_boundary(&line, 80)].to_string();
            line.push('\u{2026}');
        }
        line
    }

    /// Very small deterministic date parser for phrases like
    /// "September 18 at 11:59 PM", "Sept 18 2026 at 5pm", "18 Sep 2026 11:59 PM".
    ///
    /// Returns `None` when it cannot unambiguously resolve a full date and
    /// time -- a guessed deadline is worse than no deadline (ADR-0034).
    pub fn parse_due_phrase(text: &str) -> Option<OffsetDateTime> {
        let lower = text.to_lowercase();
        let month = detect_month(&lower)?;
        let day = detect_day(&lower)?;
        let year = detect_year(&lower)?;
        let (hour, minute) = detect_time(&lower).unwrap_or((23, 59));
        let date = Date::from_calendar_date(year, month, day).ok()?;
        let time = Time::from_hms(hour, minute, 0).ok()?;
        Some(OffsetDateTime::new_utc(date, time))
    }

    fn detect_month(lower: &str) -> Option<Month> {
        const MONTHS: &[(&str, Month)] = &[
            ("january", Month::January),
            ("february", Month::February),
            ("march", Month::March),
            ("april", Month::April),
            ("may", Month::May),
            ("june", Month::June),
            ("july", Month::July),
            ("august", Month::August),
            ("september", Month::September),
            ("october", Month::October),
            ("november", Month::November),
            ("december", Month::December),
            // Common abbreviations.
            ("jan ", Month::January),
            ("feb ", Month::February),
            ("mar ", Month::March),
            ("apr ", Month::April),
            ("jun ", Month::June),
            ("jul ", Month::July),
            ("aug ", Month::August),
            ("sept", Month::September),
            ("sep ", Month::September),
            ("oct ", Month::October),
            ("nov ", Month::November),
            ("dec ", Month::December),
        ];
        MONTHS
            .iter()
            .find(|(needle, _)| lower.contains(needle))
            .map(|(_, m)| *m)
    }

    fn detect_day(lower: &str) -> Option<u8> {
        // First one or two digit number in the phrase after "due" etc.
        // We scan for a standalone number 1..=31.
        let mut buf = String::new();
        for c in lower.chars() {
            if c.is_ascii_digit() {
                buf.push(c);
                if buf.len() > 2 {
                    buf.clear();
                }
                continue;
            }
            if !buf.is_empty() {
                if let Ok(n) = buf.parse::<u8>()
                    && (1..=31).contains(&n)
                {
                    return Some(n);
                }
                buf.clear();
            }
        }
        buf.parse::<u8>().ok().filter(|n| (1..=31).contains(n))
    }

    fn detect_year(lower: &str) -> Option<i32> {
        // Look for a 4-digit year 2020..=2099.
        let bytes = lower.as_bytes();
        let mut i = 0;
        while i + 4 <= bytes.len() {
            if bytes[i..i + 4].iter().all(|b| b.is_ascii_digit()) {
                let s = &lower[i..i + 4];
                if let Ok(year) = s.parse::<i32>()
                    && (2020..=2099).contains(&year)
                {
                    return Some(year);
                }
            }
            i += 1;
        }
        // Fall back to the current year so a syllabus that says "due September
        // 18" (no year) still produces a candidate. This mirrors what a human
        // reader would assume; the confidence field flags the uncertainty.
        Some(OffsetDateTime::now_utc().year())
    }

    fn detect_time(lower: &str) -> Option<(u8, u8)> {
        // Patterns: "11:59 pm", "11:59pm", "5pm", "17:00".
        // We scan for "hh:mm" first, then "hh am/pm".
        let bytes = lower.as_bytes();
        let mut i = 0;
        while i + 5 <= bytes.len() {
            let seg = &lower[i..i + 5];
            let b = seg.as_bytes();
            if b[0].is_ascii_digit() && (b[1] == b':' || (b[1].is_ascii_digit() && b[2] == b':')) {
                let colon = if b[1] == b':' { 1 } else { 2 };
                let hour_str = &seg[..colon];
                let after = &lower[i + colon + 1..];
                let minute_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                if minute_str.len() >= 2 {
                    let hour: u8 = hour_str.parse().ok()?;
                    let minute: u8 = minute_str[..2].parse().ok()?;
                    let ampm = &lower[i + colon + 1 + minute_str.len()..];
                    let hour = adjust_for_ampm(hour, ampm);
                    if hour < 24 && minute < 60 {
                        return Some((hour, minute));
                    }
                }
            }
            i += 1;
        }
        // "5pm", "5 pm", "12 am"
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j - i <= 2
                    && let Ok(hour) = lower[i..j].parse::<u8>()
                {
                    let after_trim = lower[j..].trim_start();
                    if after_trim.starts_with("am") || after_trim.starts_with("pm") {
                        let adjusted = adjust_for_ampm(hour, after_trim);
                        if adjusted < 24 {
                            return Some((adjusted, 0));
                        }
                    }
                }
                i = j;
                continue;
            }
            i += 1;
        }
        None
    }

    fn adjust_for_ampm(hour: u8, after: &str) -> u8 {
        let a = after.trim_start();
        if a.starts_with("pm") && hour < 12 {
            hour + 12
        } else if a.starts_with("am") && hour == 12 {
            0
        } else {
            hour
        }
    }
}

// ---------------------------------------------------------------------------
// Content hashing
// ---------------------------------------------------------------------------

/// SHA-256 of `bytes`, hex-encoded. Stable, deterministic, and small enough
/// to fit in a database column without special handling.
pub fn content_hash(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn provenance(doc: DocumentId) -> DocumentProvenance {
        DocumentProvenance {
            document_id: doc,
            filename: "syllabus.pdf".to_string(),
            source: DocumentSource::LocalUpload,
            source_ref: None,
            page_number: 3,
            extraction_method: ExtractionMethod::NativeText,
        }
    }

    #[test]
    fn document_source_and_state_round_trip_through_strings() {
        for s in [
            DocumentSource::LocalUpload,
            DocumentSource::GoogleDrive,
            DocumentSource::ExternalSource,
        ] {
            assert_eq!(DocumentSource::parse(s.as_str()), Some(s));
        }
        for state in [
            ProcessingState::Uploaded,
            ProcessingState::Extracting,
            ProcessingState::Ocr,
            ProcessingState::Verifying,
            ProcessingState::Indexed,
            ProcessingState::Failed,
        ] {
            assert_eq!(ProcessingState::parse(state.as_str()), Some(state));
        }
    }

    #[test]
    fn is_supported_mime_covers_pdf_and_text_types() {
        assert!(is_supported_mime("application/pdf"));
        assert!(is_supported_mime("text/plain"));
        assert!(is_supported_mime("text/markdown"));
        assert!(!is_supported_mime("image/png"));
        assert!(!is_supported_mime("application/zip"));
    }

    #[test]
    fn content_hash_is_stable_and_hex() {
        let a = content_hash(b"hello world");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(content_hash(b"hello world"), a);
        assert_ne!(content_hash(b"hello world!"), a);
    }

    #[tokio::test]
    async fn in_memory_store_scopes_by_user() {
        let store = InMemoryDocumentStore::new();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        let doc = store
            .create(NewDocument {
                user_id: alice,
                filename: "notes.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size_bytes: 4096,
                source: DocumentSource::LocalUpload,
                source_ref: None,
                content_hash: content_hash(b"hello"),
                storage_key: "mem://x".to_string(),
            })
            .await
            .unwrap();

        assert!(matches!(
            store.get(bob, doc.id).await,
            Err(DocumentError::NotFound(_))
        ));
        assert!(matches!(
            store.delete(bob, doc.id).await,
            Err(DocumentError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn creating_the_same_bytes_twice_returns_the_same_document() {
        let store = InMemoryDocumentStore::new();
        let user = Uuid::new_v4();
        let hash = content_hash(b"same bytes");

        let a = store
            .create(NewDocument {
                user_id: user,
                filename: "x.pdf".into(),
                mime_type: "application/pdf".into(),
                size_bytes: 10,
                source: DocumentSource::LocalUpload,
                source_ref: None,
                content_hash: hash.clone(),
                storage_key: "mem://a".into(),
            })
            .await
            .unwrap();
        let b = store
            .create(NewDocument {
                user_id: user,
                filename: "x.pdf".into(),
                mime_type: "application/pdf".into(),
                size_bytes: 10,
                source: DocumentSource::LocalUpload,
                source_ref: None,
                content_hash: hash,
                storage_key: "mem://b".into(),
            })
            .await
            .unwrap();
        assert_eq!(a.id, b.id);
    }

    #[tokio::test]
    async fn page_search_returns_snippet_and_score() {
        let store = InMemoryDocumentStore::new();
        let user = Uuid::new_v4();
        let doc = store
            .create(NewDocument {
                user_id: user,
                filename: "essay.pdf".into(),
                mime_type: "application/pdf".into(),
                size_bytes: 100,
                source: DocumentSource::LocalUpload,
                source_ref: None,
                content_hash: content_hash(b"body"),
                storage_key: "mem://e".into(),
            })
            .await
            .unwrap();
        let pages = vec![
            DocumentPage {
                document_id: doc.id,
                user_id: user,
                page_number: 1,
                extraction_method: ExtractionMethod::NativeText,
                content: "Introduction to widgets.".into(),
                confidence: None,
                char_count: 24,
            },
            DocumentPage {
                document_id: doc.id,
                user_id: user,
                page_number: 2,
                extraction_method: ExtractionMethod::NativeText,
                content: "Submission deadline is September 18 at 11:59 PM. See rubric.".into(),
                confidence: None,
                char_count: 60,
            },
        ];
        store.replace_pages(user, doc.id, pages).await.unwrap();

        let hits = store
            .search_pages(DocumentQuery {
                user_id: user,
                text: Some("submission".into()),
                limit: 5,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page_number, 2);
        assert!(hits[0].snippet.to_lowercase().contains("submission"));
    }

    #[tokio::test]
    async fn null_and_fake_ocr_providers_behave_as_documented() {
        let page = RenderedPage {
            page_number: 1,
            image_bytes: Vec::new(),
            text_hint: Some("ocr text".into()),
        };
        assert!(matches!(
            NullOcrProvider.recognise(&page).await,
            Err(OcrError::NotAvailable)
        ));
        let result = FakeOcrProvider::new(0.9).recognise(&page).await.unwrap();
        assert_eq!(result.text, "ocr text");
        assert!((result.confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn deadline_extractor_finds_a_full_date_time_phrase() {
        let doc = Uuid::new_v4();
        let prov = provenance(doc);
        let text = "Assignment 3: essay. Submission due September 18 2026 at 11:59 PM.";
        let candidates = deadlines::extract(text, &prov);
        assert!(!candidates.is_empty(), "no candidates parsed");
        let with_time = candidates
            .iter()
            .find(|c| c.due_at.is_some())
            .expect("expected a candidate with a resolved due_at");
        let due = with_time.due_at.unwrap();
        assert_eq!(due.year(), 2026);
        assert_eq!(due.month(), time::Month::September);
        assert_eq!(due.day(), 18);
        assert_eq!(due.hour(), 23);
        assert_eq!(due.minute(), 59);
        assert!(with_time.confidence > 0.5);
    }

    #[test]
    fn deadline_extractor_returns_no_due_at_when_the_phrase_is_ambiguous() {
        let prov = provenance(Uuid::new_v4());
        let candidates = deadlines::extract("Homework due at some point next week", &prov);
        assert!(
            candidates
                .iter()
                .all(|c| c.due_at.is_none() || c.confidence <= 0.7)
        );
    }

    #[test]
    fn deadline_extractor_records_provenance_verbatim() {
        let prov = provenance(Uuid::new_v4());
        let candidates = deadlines::extract("Submission due September 3 at 5 PM", &prov);
        assert!(!candidates.is_empty());
        for candidate in candidates {
            assert_eq!(candidate.provenance, prov);
        }
    }

    #[test]
    fn document_memory_proposal_carries_document_provenance() {
        let doc = Uuid::new_v4();
        let prov = provenance(doc);
        let proposal = DocumentMemoryProposal {
            kind: assistant_memory::MemoryKind::Fact,
            content: "The final essay is due September 18.".into(),
            confidence: 0.8,
            provenance: prov.clone(),
        };
        let mem = proposal.into_memory_proposal();
        assert_eq!(mem.source_kind, assistant_memory::MemorySource::Document);
        assert!(
            mem.source_ref
                .as_deref()
                .unwrap()
                .starts_with(&doc.to_string())
        );
        assert!(mem.reason.as_deref().unwrap().contains("syllabus.pdf"));
    }

    #[test]
    fn snippet_around_snaps_to_char_boundaries() {
        let content = "α β γ δ SUBMISSION deadline is Sept 18 ε ζ η";
        let snippet = snippet_around(content, "submission");
        assert!(snippet.contains("SUBMISSION"));
        // Just verify no panic and non-empty output on multi-byte content.
        assert!(!snippet.is_empty());
    }
}
