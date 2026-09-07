//! Postgres implementations of the M8 storage traits.
//!
//! Same rule as every other Postgres impl in this crate: ownership is
//! enforced in SQL, not by a Rust-side check that runs after the read. A
//! document belonging to somebody else must be indistinguishable from a
//! document that does not exist.

use assistant_documents::{
    Document, DocumentError, DocumentId, DocumentPage, DocumentPatch, DocumentQuery,
    DocumentSource, DocumentStorage, DocumentStore, ExtractionMethod, NewDocument, PageSearchHit,
    ProcessingState, SEARCH_SNIPPET_CHARS, StorageError, UserId, snippet_around,
};
use async_trait::async_trait;
use sqlx::{PgPool, Row, postgres::PgRow};
use uuid::Uuid;

pub struct PostgresDocumentStore {
    pool: PgPool,
}

impl PostgresDocumentStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn backend(error: sqlx::Error) -> DocumentError {
    DocumentError::Backend(error.to_string())
}

fn document_from_row(row: &PgRow) -> Result<Document, DocumentError> {
    let source: String = row.try_get("source").map_err(backend)?;
    let source = DocumentSource::parse(&source)
        .ok_or_else(|| DocumentError::Backend(format!("unknown source: {source}")))?;

    let state: String = row.try_get("processing_state").map_err(backend)?;
    let state = ProcessingState::parse(&state)
        .ok_or_else(|| DocumentError::Backend(format!("unknown state: {state}")))?;

    let size: i64 = row.try_get("size_bytes").map_err(backend)?;
    let page_count: Option<i32> = row.try_get("page_count").map_err(backend)?;

    Ok(Document {
        id: row.try_get("id").map_err(backend)?,
        user_id: row.try_get("user_id").map_err(backend)?,
        filename: row.try_get("filename").map_err(backend)?,
        mime_type: row.try_get("mime_type").map_err(backend)?,
        size_bytes: size.max(0) as u64,
        source,
        source_ref: row.try_get("source_ref").map_err(backend)?,
        content_hash: row.try_get("content_hash").map_err(backend)?,
        page_count: page_count.map(|n| n.max(0) as u32),
        processing_state: state,
        processing_error: row.try_get("processing_error").map_err(backend)?,
        storage_key: row.try_get("storage_key").map_err(backend)?,
        created_at: row.try_get("created_at").map_err(backend)?,
        updated_at: row.try_get("updated_at").map_err(backend)?,
        processed_at: row.try_get("processed_at").map_err(backend)?,
    })
}

fn page_from_row(row: &PgRow) -> Result<DocumentPage, DocumentError> {
    let method: String = row.try_get("extraction_method").map_err(backend)?;
    let method = ExtractionMethod::parse(&method)
        .ok_or_else(|| DocumentError::Backend(format!("unknown extraction method: {method}")))?;
    let page_number: i32 = row.try_get("page_number").map_err(backend)?;
    let char_count: i32 = row.try_get("char_count").map_err(backend)?;
    Ok(DocumentPage {
        document_id: row.try_get("document_id").map_err(backend)?,
        user_id: row.try_get("user_id").map_err(backend)?,
        page_number: page_number.max(0) as u32,
        extraction_method: method,
        content: row.try_get("content").map_err(backend)?,
        confidence: row.try_get("confidence").map_err(backend)?,
        char_count: char_count.max(0) as u32,
    })
}

const DOCUMENT_COLUMNS: &str = "id, user_id, filename, mime_type, size_bytes, source,
    source_ref, content_hash, page_count, processing_state, processing_error,
    storage_key, created_at, updated_at, processed_at";

const PAGE_COLUMNS: &str = "document_id, user_id, page_number, extraction_method,
    content, confidence, char_count";

async fn ensure_user(pool: &PgPool, user_id: Uuid) -> Result<(), DocumentError> {
    sqlx::query("insert into users (id) values ($1) on conflict (id) do nothing")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(backend)?;
    Ok(())
}

#[async_trait]
impl DocumentStore for PostgresDocumentStore {
    /// Insert-or-return: `(user_id, content_hash)` is unique, so the same
    /// bytes uploaded twice for the same user resolve to the same row.
    async fn create(&self, new_document: NewDocument) -> Result<Document, DocumentError> {
        ensure_user(&self.pool, new_document.user_id).await?;

        let mut tx = self.pool.begin().await.map_err(backend)?;

        // On conflict (same user, same bytes) refresh the storage_key: the
        // caller has just written a fresh copy of the bytes, and pointing at
        // that fresh copy means the pipeline will not fail if the original
        // storage entry has been garbage-collected. The old storage entry
        // becomes an orphan, which storage-side GC handles separately.
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into documents (
                 user_id, filename, mime_type, size_bytes, source, source_ref,
                 content_hash, storage_key
             ) values ($1, $2, $3, $4, $5, $6, $7, $8)
             on conflict (user_id, content_hash) do update
                 set updated_at = now(),
                     storage_key = excluded.storage_key
             returning {DOCUMENT_COLUMNS}"
        )))
        .bind(new_document.user_id)
        .bind(&new_document.filename)
        .bind(&new_document.mime_type)
        .bind(new_document.size_bytes as i64)
        .bind(new_document.source.as_str())
        .bind(new_document.source_ref.as_deref())
        .bind(&new_document.content_hash)
        .bind(&new_document.storage_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(backend)?;

        let stored = document_from_row(&row)?;
        tx.commit().await.map_err(backend)?;
        Ok(stored)
    }

    async fn get(&self, user_id: UserId, id: DocumentId) -> Result<Document, DocumentError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {DOCUMENT_COLUMNS} from documents where id = $1 and user_id = $2"
        )))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        match row {
            Some(row) => document_from_row(&row),
            None => Err(DocumentError::NotFound(id)),
        }
    }

    async fn search(&self, query: DocumentQuery) -> Result<Vec<Document>, DocumentError> {
        let limit = query.effective_limit() as i64;
        let text: Option<String> = query
            .text
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let states: Option<Vec<String>> = if query.states.is_empty() {
            None
        } else {
            Some(
                query
                    .states
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect(),
            )
        };

        // The text predicate is satisfied by a match on the filename, on any
        // page's tsvector, or a plain ilike on any page's content (for short
        // substring hits like "sept").
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select distinct {columns}
             from documents d
             left join document_pages p on p.document_id = d.id and p.user_id = d.user_id
             where d.user_id = $1
               and ($2::text is null or d.mime_type = $2)
               and ($3::text[] is null or d.processing_state = any($3::text[]))
               and (
                   $4::text is null
                   or d.filename ilike '%' || $4::text || '%'
                   or p.search_tsv @@ websearch_to_tsquery('pg_catalog.english', $4::text)
                   or p.content ilike '%' || $4::text || '%'
               )
             order by d.updated_at desc
             limit $5",
            columns = DOCUMENT_COLUMNS
                .split(',')
                .map(|c| format!("d.{}", c.trim()))
                .collect::<Vec<_>>()
                .join(", ")
        )))
        .bind(query.user_id)
        .bind(query.mime_type.as_deref())
        .bind(states)
        .bind(text)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        rows.iter().map(document_from_row).collect()
    }

    async fn update(
        &self,
        user_id: UserId,
        id: DocumentId,
        patch: DocumentPatch,
    ) -> Result<Document, DocumentError> {
        // Same "coalesce is 'leave alone'" trick used in productivity and
        // memory. For the two double-option fields, a `Some(None)` clears
        // and a `Some(Some(_))` sets.
        let error_clear = matches!(patch.processing_error, Some(None));
        let error_set = patch.processing_error.flatten();
        let processed_clear = matches!(patch.processed_at, Some(None));
        let processed_set = patch.processed_at.flatten();

        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "update documents set
                 processing_state = coalesce($1, processing_state),
                 processing_error = case
                                       when $2::bool then null
                                       when $3::text is not null then $3
                                       else processing_error
                                    end,
                 page_count       = coalesce($4::integer, page_count),
                 processed_at     = case
                                       when $5::bool then null
                                       when $6::timestamptz is not null then $6
                                       else processed_at
                                    end
             where id = $7 and user_id = $8
             returning {DOCUMENT_COLUMNS}"
        )))
        .bind(patch.processing_state.map(|s| s.as_str().to_string()))
        .bind(error_clear)
        .bind(error_set.as_deref())
        .bind(patch.page_count.map(|n| n as i32))
        .bind(processed_clear)
        .bind(processed_set)
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        match row {
            Some(row) => document_from_row(&row),
            None => Err(DocumentError::NotFound(id)),
        }
    }

    async fn delete(&self, user_id: UserId, id: DocumentId) -> Result<(), DocumentError> {
        let result = sqlx::query("delete from documents where id = $1 and user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(backend)?;
        if result.rows_affected() == 0 {
            return Err(DocumentError::NotFound(id));
        }
        Ok(())
    }

    async fn replace_pages(
        &self,
        user_id: UserId,
        id: DocumentId,
        pages: Vec<DocumentPage>,
    ) -> Result<(), DocumentError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;

        // Ownership check up front. Doing the check as a SELECT inside the
        // transaction keeps a caller from silently no-oping against another
        // user's document.
        let owned: Option<(Uuid,)> =
            sqlx::query_as("select id from documents where id = $1 and user_id = $2")
                .bind(id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(backend)?;
        if owned.is_none() {
            return Err(DocumentError::NotFound(id));
        }

        sqlx::query("delete from document_pages where document_id = $1 and user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

        for page in pages {
            if page.document_id != id || page.user_id != user_id {
                return Err(DocumentError::Invalid(
                    "page ownership does not match its parent document".into(),
                ));
            }
            sqlx::query(
                "insert into document_pages
                     (document_id, user_id, page_number, extraction_method,
                      content, confidence, char_count)
                 values ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(page.document_id)
            .bind(page.user_id)
            .bind(page.page_number as i32)
            .bind(page.extraction_method.as_str())
            .bind(&page.content)
            .bind(page.confidence)
            .bind(page.char_count as i32)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        }

        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    async fn pages(
        &self,
        user_id: UserId,
        id: DocumentId,
    ) -> Result<Vec<DocumentPage>, DocumentError> {
        // First confirm the parent belongs to this user, so an unauthorised
        // caller cannot even learn how many pages exist.
        let owned: Option<(Uuid,)> =
            sqlx::query_as("select id from documents where id = $1 and user_id = $2")
                .bind(id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(backend)?;
        if owned.is_none() {
            return Err(DocumentError::NotFound(id));
        }

        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {PAGE_COLUMNS} from document_pages
             where document_id = $1 and user_id = $2
             order by page_number asc"
        )))
        .bind(id)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        rows.iter().map(page_from_row).collect()
    }

    async fn get_page(
        &self,
        user_id: UserId,
        id: DocumentId,
        page_number: u32,
    ) -> Result<DocumentPage, DocumentError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {PAGE_COLUMNS} from document_pages
             where document_id = $1 and user_id = $2 and page_number = $3"
        )))
        .bind(id)
        .bind(user_id)
        .bind(page_number as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        match row {
            Some(row) => page_from_row(&row),
            None => Err(DocumentError::PageNotFound {
                document: id,
                page: page_number,
            }),
        }
    }

    async fn search_pages(
        &self,
        query: DocumentQuery,
    ) -> Result<Vec<PageSearchHit>, DocumentError> {
        let Some(text) = query
            .text
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        else {
            return Ok(Vec::new());
        };
        let limit = query.effective_limit() as i64;

        let rows = sqlx::query(
            "select d.filename, p.document_id, p.page_number, p.extraction_method,
                    p.content,
                    ts_rank(p.search_tsv, websearch_to_tsquery('pg_catalog.english', $2))
                        as tsrank
             from document_pages p
             join documents d on d.id = p.document_id and d.user_id = p.user_id
             where p.user_id = $1
               and (
                   p.search_tsv @@ websearch_to_tsquery('pg_catalog.english', $2)
                   or p.content ilike '%' || $2 || '%'
               )
             order by tsrank desc
             limit $3",
        )
        .bind(query.user_id)
        .bind(&text)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        let text_lc = text.to_lowercase();
        let hits: Vec<PageSearchHit> = rows
            .iter()
            .map(|row| -> Result<PageSearchHit, DocumentError> {
                let method: String = row.try_get("extraction_method").map_err(backend)?;
                let extraction_method = ExtractionMethod::parse(&method).ok_or_else(|| {
                    DocumentError::Backend(format!("unknown extraction method: {method}"))
                })?;
                let content: String = row.try_get("content").map_err(backend)?;
                let page_number: i32 = row.try_get("page_number").map_err(backend)?;
                let tsrank: f32 = row.try_get("tsrank").map_err(backend)?;
                let snippet = snippet_around(&content, &text_lc);
                let snippet = if snippet.chars().count() > SEARCH_SNIPPET_CHARS {
                    snippet.chars().take(SEARCH_SNIPPET_CHARS).collect()
                } else {
                    snippet
                };
                Ok(PageSearchHit {
                    document_id: row.try_get("document_id").map_err(backend)?,
                    filename: row.try_get("filename").map_err(backend)?,
                    page_number: page_number.max(0) as u32,
                    extraction_method,
                    snippet,
                    score: tsrank.max(0.0),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(hits)
    }
}

// ---------------------------------------------------------------------------
// LocalFilesystemStorage
// ---------------------------------------------------------------------------

/// Stores each document as a file under `<root>/<user_id>/<document_id>`.
///
/// Suitable for a single-node deployment or a test. Production instances that
/// need multi-node access should plug in a Supabase Storage or S3 backend
/// against the same trait; both are straightforward HTTP clients and neither
/// requires a database schema change here.
pub struct LocalFilesystemStorage {
    root: std::path::PathBuf,
}

impl LocalFilesystemStorage {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl DocumentStorage for LocalFilesystemStorage {
    async fn write(
        &self,
        user_id: UserId,
        document_id: DocumentId,
        _content_type: &str,
        bytes: &[u8],
    ) -> Result<String, StorageError> {
        let dir = self.root.join(user_id.to_string());
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| StorageError::Write(e.to_string()))?;
        let path = dir.join(document_id.to_string());
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| StorageError::Write(e.to_string()))?;
        // Storage key is the relative path from the root. Absolute path
        // would leak the deployment layout in API responses and logs; the
        // relative key stays stable across a config move.
        let key = format!("local://{}/{}", user_id, document_id);
        tracing::debug!(path = %path.display(), key = %key, "wrote document bytes");
        Ok(key)
    }

    async fn read(&self, storage_key: &str) -> Result<Vec<u8>, StorageError> {
        let (user, doc) = parse_local_key(storage_key)
            .ok_or_else(|| StorageError::Read(format!("bad storage key: {storage_key}")))?;
        let path = self.root.join(&user).join(&doc);
        tracing::debug!(path = %path.display(), key = %storage_key, "reading document bytes");
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(StorageError::NotFound),
            Err(err) => Err(StorageError::Read(err.to_string())),
        }
    }

    async fn delete(&self, storage_key: &str) -> Result<(), StorageError> {
        let (user, doc) = parse_local_key(storage_key)
            .ok_or_else(|| StorageError::Delete(format!("bad storage key: {storage_key}")))?;
        let path = self.root.join(user).join(doc);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StorageError::Delete(err.to_string())),
        }
    }
}

fn parse_local_key(key: &str) -> Option<(String, String)> {
    let rest = key.strip_prefix("local://")?;
    let (user, doc) = rest.split_once('/')?;
    // A rogue caller must never traverse out of the root. UUIDs never contain
    // "/" or ".."; refusing anything else keeps this literal.
    if user.contains("..") || doc.contains("..") || user.contains('/') || doc.contains('/') {
        return None;
    }
    Some((user.to_string(), doc.to_string()))
}
