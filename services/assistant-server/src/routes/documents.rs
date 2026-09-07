//! REST endpoints for M8 document / PDF intelligence.
//!
//! What lives here: the HTTP surface. The pipeline itself is
//! `assistant-documents::pipeline::Pipeline`; this file only translates
//! requests into pipeline calls and translates responses into wire types.
//!
//! Ownership always comes from the authenticated principal. A client cannot
//! send a `user_id` and cannot read another user's documents even by id --
//! the store returns `NotFound` for anything not owned by the caller.

use assistant_auth::Principal;
use assistant_documents::{
    Document, DocumentError, DocumentPage, DocumentQuery, DocumentSource, ExtractionMethod,
    MAX_DOCUMENT_BYTES, NewDocument, PageSearchHit, ProcessingState, content_hash,
    is_supported_mime, pipeline::Pipeline,
};
use assistant_protocol::{
    DocumentItem, DocumentPageItem, DocumentProcessingStateDto, DocumentSearchHit,
    DocumentSourceDto, ExtractionMethodDto, IngestFromDriveRequest,
};
use assistant_tools::DriveProvider;
use axum::http::HeaderMap;
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::header,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{error::AppError, state::SharedState};

fn pipeline(state: &SharedState) -> Result<Pipeline, AppError> {
    state
        .documents
        .clone()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("document pipeline unavailable")))
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

fn source_to(source: DocumentSource) -> DocumentSourceDto {
    match source {
        DocumentSource::LocalUpload => DocumentSourceDto::LocalUpload,
        DocumentSource::GoogleDrive => DocumentSourceDto::GoogleDrive,
        DocumentSource::ExternalSource => DocumentSourceDto::ExternalSource,
    }
}

fn state_to(state: ProcessingState) -> DocumentProcessingStateDto {
    match state {
        ProcessingState::Uploaded => DocumentProcessingStateDto::Uploaded,
        ProcessingState::Extracting => DocumentProcessingStateDto::Extracting,
        ProcessingState::Ocr => DocumentProcessingStateDto::Ocr,
        ProcessingState::Verifying => DocumentProcessingStateDto::Verifying,
        ProcessingState::Indexed => DocumentProcessingStateDto::Indexed,
        ProcessingState::Failed => DocumentProcessingStateDto::Failed,
    }
}

fn state_from(state: DocumentProcessingStateDto) -> ProcessingState {
    match state {
        DocumentProcessingStateDto::Uploaded => ProcessingState::Uploaded,
        DocumentProcessingStateDto::Extracting => ProcessingState::Extracting,
        DocumentProcessingStateDto::Ocr => ProcessingState::Ocr,
        DocumentProcessingStateDto::Verifying => ProcessingState::Verifying,
        DocumentProcessingStateDto::Indexed => ProcessingState::Indexed,
        DocumentProcessingStateDto::Failed => ProcessingState::Failed,
    }
}

fn method_to(method: ExtractionMethod) -> ExtractionMethodDto {
    match method {
        ExtractionMethod::NativeText => ExtractionMethodDto::NativeText,
        ExtractionMethod::Ocr => ExtractionMethodDto::Ocr,
        ExtractionMethod::VisualVerification => ExtractionMethodDto::VisualVerification,
        ExtractionMethod::None => ExtractionMethodDto::None,
    }
}

fn to_item(doc: Document) -> DocumentItem {
    DocumentItem {
        id: doc.id,
        user_id: doc.user_id,
        filename: doc.filename,
        mime_type: doc.mime_type,
        size_bytes: doc.size_bytes,
        source: source_to(doc.source),
        source_ref: doc.source_ref,
        content_hash: doc.content_hash,
        page_count: doc.page_count,
        processing_state: state_to(doc.processing_state),
        processing_error: doc.processing_error,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
        processed_at: doc.processed_at,
    }
}

fn to_page(page: DocumentPage) -> DocumentPageItem {
    DocumentPageItem {
        document_id: page.document_id,
        user_id: page.user_id,
        page_number: page.page_number,
        extraction_method: method_to(page.extraction_method),
        content: page.content,
        confidence: page.confidence,
        char_count: page.char_count,
    }
}

fn to_hit(hit: PageSearchHit) -> DocumentSearchHit {
    DocumentSearchHit {
        document_id: hit.document_id,
        filename: hit.filename,
        page_number: hit.page_number,
        extraction_method: method_to(hit.extraction_method),
        snippet: hit.snippet,
        score: hit.score,
    }
}

fn map_document_error(error: DocumentError) -> AppError {
    match error {
        DocumentError::NotFound(_) | DocumentError::PageNotFound { .. } => AppError::NotFound,
        DocumentError::Invalid(msg) => AppError::BadRequest(msg),
        DocumentError::UnsupportedType(mime) => {
            AppError::BadRequest(format!("unsupported document type: {mime}"))
        }
        DocumentError::TooLarge { size, limit } => AppError::BadRequest(format!(
            "document is {size} bytes; the limit is {limit} bytes"
        )),
        DocumentError::Backend(msg) => AppError::Internal(anyhow::anyhow!(msg)),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/documents
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Comma-separated list; the parser mirrors the memory route.
    #[serde(default, deserialize_with = "csv_states")]
    pub state: Vec<DocumentProcessingStateDto>,
    #[serde(default)]
    pub limit: Option<u32>,
}

fn csv_states<'de, D>(deserializer: D) -> Result<Vec<DocumentProcessingStateDto>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Option<String> = Option::deserialize(deserializer)?;
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .map(|piece| {
            serde_json::from_value::<DocumentProcessingStateDto>(serde_json::Value::String(
                piece.to_string(),
            ))
            .map_err(serde::de::Error::custom)
        })
        .collect()
}

pub async fn list_documents(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<DocumentItem>>, AppError> {
    let pipe = pipeline(&state)?;
    let query = DocumentQuery {
        user_id: principal.user_id,
        text: params.q,
        mime_type: params.mime_type,
        states: params.state.into_iter().map(state_from).collect(),
        limit: params.limit.unwrap_or(0),
    };
    let docs = pipe.store.search(query).await.map_err(map_document_error)?;
    Ok(Json(docs.into_iter().map(to_item).collect()))
}

// ---------------------------------------------------------------------------
// GET /v1/documents/{id}
// ---------------------------------------------------------------------------

pub async fn get_document(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<DocumentItem>, AppError> {
    let pipe = pipeline(&state)?;
    let doc = pipe
        .store
        .get(principal.user_id, id)
        .await
        .map_err(map_document_error)?;
    Ok(Json(to_item(doc)))
}

// ---------------------------------------------------------------------------
// POST /v1/documents (raw body upload)
// ---------------------------------------------------------------------------
//
// The client sends the raw file body with `Content-Type` set to the file's
// MIME type and `X-Filename` set to the display name. This is deliberately
// not a multipart form: multipart adds a parser, a boundary marshalling
// layer, and a codepath that would let a client claim more than one file per
// request. A body plus two headers is enough.

pub async fn upload_document(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<DocumentItem>, AppError> {
    let pipe = pipeline(&state)?;

    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("missing Content-Type header".into()))?;

    if !is_supported_mime(&mime) {
        return Err(map_document_error(DocumentError::UnsupportedType(mime)));
    }

    let filename = headers
        .get("x-filename")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| suggested_filename(&mime));

    if body.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err(map_document_error(DocumentError::TooLarge {
            size: body.len() as u64,
            limit: MAX_DOCUMENT_BYTES,
        }));
    }
    if body.is_empty() {
        return Err(AppError::BadRequest("document body is empty".into()));
    }

    ingest_and_process(
        &pipe,
        principal.user_id,
        &filename,
        &mime,
        DocumentSource::LocalUpload,
        None,
        &body,
    )
    .await
    .map(Json)
}

fn suggested_filename(mime: &str) -> String {
    match mime {
        "application/pdf" => "document.pdf".into(),
        "text/plain" => "note.txt".into(),
        "text/markdown" => "note.md".into(),
        "text/csv" => "sheet.csv".into(),
        "text/html" => "page.html".into(),
        "application/json" => "data.json".into(),
        "application/xml" | "text/xml" => "data.xml".into(),
        _ => "document".into(),
    }
}

async fn ingest_and_process(
    pipe: &Pipeline,
    user_id: Uuid,
    filename: &str,
    mime: &str,
    source: DocumentSource,
    source_ref: Option<String>,
    bytes: &[u8],
) -> Result<DocumentItem, AppError> {
    let hash = content_hash(bytes);
    let temp_id = Uuid::new_v4();
    let storage_key = pipe
        .storage
        .write(user_id, temp_id, mime, bytes)
        .await
        .map_err(|error| AppError::Internal(anyhow::anyhow!("storage write: {error}")))?;
    let created = pipe
        .store
        .create(NewDocument {
            user_id,
            filename: filename.to_string(),
            mime_type: mime.to_string(),
            size_bytes: bytes.len() as u64,
            source,
            source_ref,
            content_hash: hash,
            storage_key,
        })
        .await
        .map_err(map_document_error)?;

    // Process synchronously so the client gets the final state in one round
    // trip. Extraction is fast for M8's supported types; heavier pipelines
    // (large scanned PDFs, real OCR) will need a background job. That belongs
    // to a later milestone and is called out in ADR-0036.
    let final_doc = pipe.process(&created).await.map_err(map_document_error)?;
    Ok(to_item(final_doc))
}

// ---------------------------------------------------------------------------
// POST /v1/documents/from-drive
// ---------------------------------------------------------------------------

pub async fn ingest_from_drive(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<IngestFromDriveRequest>,
) -> Result<Json<DocumentItem>, AppError> {
    let pipe = pipeline(&state)?;
    let google = state
        .google
        .clone()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("google client unavailable")))?;

    // Metadata first; refuses folders and lets us decide on size before we
    // download anything.
    let meta = google
        .metadata(input.account_id, principal.user_id, &input.file_id)
        .await
        .map_err(|error| AppError::BadRequest(format!("drive metadata: {error}")))?;

    if meta.is_folder {
        return Err(AppError::BadRequest("that is a folder, not a file".into()));
    }
    if !is_supported_mime(&meta.mime_type) {
        return Err(map_document_error(DocumentError::UnsupportedType(
            meta.mime_type.clone(),
        )));
    }
    if let Some(size) = meta.size_bytes
        && size > MAX_DOCUMENT_BYTES
    {
        return Err(map_document_error(DocumentError::TooLarge {
            size,
            limit: MAX_DOCUMENT_BYTES,
        }));
    }

    let bytes = crate::google::drive::download_document_bytes(
        &google,
        input.account_id,
        principal.user_id,
        &input.file_id,
        MAX_DOCUMENT_BYTES,
    )
    .await
    .map_err(|error| AppError::BadRequest(format!("drive download: {error}")))?;

    ingest_and_process(
        &pipe,
        principal.user_id,
        &meta.name,
        &meta.mime_type,
        DocumentSource::GoogleDrive,
        Some(input.file_id.clone()),
        &bytes,
    )
    .await
    .map(Json)
}

// ---------------------------------------------------------------------------
// GET /v1/documents/{id}/pages
// ---------------------------------------------------------------------------

pub async fn list_pages(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<DocumentPageItem>>, AppError> {
    let pipe = pipeline(&state)?;
    let pages = pipe
        .store
        .pages(principal.user_id, id)
        .await
        .map_err(map_document_error)?;
    Ok(Json(pages.into_iter().map(to_page).collect()))
}

// ---------------------------------------------------------------------------
// GET /v1/documents/{id}/pages/{n}
// ---------------------------------------------------------------------------

pub async fn get_page(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path((id, page_number)): Path<(Uuid, u32)>,
) -> Result<Json<DocumentPageItem>, AppError> {
    let pipe = pipeline(&state)?;
    let page = pipe
        .store
        .get_page(principal.user_id, id, page_number)
        .await
        .map_err(map_document_error)?;
    Ok(Json(to_page(page)))
}

// ---------------------------------------------------------------------------
// GET /v1/documents/search  -- page-level search
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PageSearchParams {
    pub q: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

pub async fn search_pages(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(params): Query<PageSearchParams>,
) -> Result<Json<Vec<DocumentSearchHit>>, AppError> {
    let pipe = pipeline(&state)?;
    let query = DocumentQuery {
        user_id: principal.user_id,
        text: Some(params.q),
        limit: params.limit.unwrap_or(0),
        ..Default::default()
    };
    let hits = pipe
        .store
        .search_pages(query)
        .await
        .map_err(map_document_error)?;
    Ok(Json(hits.into_iter().map(to_hit).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/documents/{id}/reprocess
// ---------------------------------------------------------------------------

pub async fn reprocess_document(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<DocumentItem>, AppError> {
    let pipe = pipeline(&state)?;
    let doc = pipe
        .store
        .get(principal.user_id, id)
        .await
        .map_err(map_document_error)?;
    let final_doc = pipe.process(&doc).await.map_err(map_document_error)?;
    Ok(Json(to_item(final_doc)))
}

// ---------------------------------------------------------------------------
// DELETE /v1/documents/{id}
// ---------------------------------------------------------------------------

pub async fn delete_document(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<(), AppError> {
    let pipe = pipeline(&state)?;
    // Read first so we know the storage key. If the row belongs to another
    // user this returns NotFound before touching the object store.
    let doc = pipe
        .store
        .get(principal.user_id, id)
        .await
        .map_err(map_document_error)?;
    pipe.store
        .delete(principal.user_id, id)
        .await
        .map_err(map_document_error)?;
    // Best-effort: an object that fails to delete is not worth surfacing as
    // an API error, but it is worth logging.
    if let Err(error) = pipe.storage.delete(&doc.storage_key).await {
        tracing::warn!(document_id = %id, error = %error, "failed to delete document bytes");
    }
    Ok(())
}
