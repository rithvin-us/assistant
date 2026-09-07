//! Postgres implementation of [`assistant_memory::MemoryStore`].
//!
//! Lives here, not in `assistant-memory`, because `sqlx` is infrastructure and
//! the domain crate depends on interfaces. Same rule as
//! `PostgresConversationStore` and `PostgresActionStore`.
//!
//! Ownership is the property this file exists to guarantee, and it is enforced
//! in SQL. Every statement scopes on `user_id`; a memory belonging to somebody
//! else must be indistinguishable from a memory that does not exist. There is
//! no path that reads a row and checks the owner afterwards -- that shape of
//! check is one people forget to write.

use assistant_memory::{
    Confidence, Importance, Lifecycle, Memory, MemoryError, MemoryId, MemoryKind, MemoryPatch,
    MemoryQuery, MemorySource, MemoryStore, NewMemory, Provenance, UserId,
};
use async_trait::async_trait;
use sqlx::{PgPool, Row, postgres::PgRow};
use time::OffsetDateTime;

/// Postgres-backed store.
pub struct PostgresMemoryStore {
    pool: PgPool,
}

impl PostgresMemoryStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn backend(error: sqlx::Error) -> MemoryError {
    MemoryError::Backend(error.to_string())
}

fn memory_from_row(row: &PgRow) -> Result<Memory, MemoryError> {
    let kind: String = row.try_get("kind").map_err(backend)?;
    let kind = MemoryKind::parse(&kind)
        .ok_or_else(|| MemoryError::Backend(format!("unknown memory kind in database: {kind}")))?;

    let lifecycle: String = row.try_get("lifecycle").map_err(backend)?;
    let lifecycle = Lifecycle::parse(&lifecycle).ok_or_else(|| {
        MemoryError::Backend(format!("unknown lifecycle in database: {lifecycle}"))
    })?;

    let source_kind: String = row.try_get("source_kind").map_err(backend)?;
    let source_kind = MemorySource::parse(&source_kind).ok_or_else(|| {
        MemoryError::Backend(format!("unknown memory source in database: {source_kind}"))
    })?;

    let importance: i16 = row.try_get("importance").map_err(backend)?;
    let importance = Importance::new(importance.clamp(1, 5) as u8)?;

    let confidence: f32 = row.try_get("confidence").map_err(backend)?;
    let confidence = Confidence::new(confidence.clamp(0.0, 1.0))?;

    let access_count: i32 = row.try_get("access_count").map_err(backend)?;

    Ok(Memory {
        id: row.try_get("id").map_err(backend)?,
        user_id: row.try_get("user_id").map_err(backend)?,
        kind,
        lifecycle,
        content: row.try_get("content").map_err(backend)?,
        importance,
        confidence,
        provenance: Provenance {
            source_kind,
            source_ref: row.try_get("source_ref").map_err(backend)?,
        },
        expires_at: row.try_get("expires_at").map_err(backend)?,
        created_at: row.try_get("created_at").map_err(backend)?,
        updated_at: row.try_get("updated_at").map_err(backend)?,
        last_accessed_at: row.try_get("last_accessed_at").map_err(backend)?,
        access_count: access_count.max(0) as u32,
        archived_at: row.try_get("archived_at").map_err(backend)?,
        superseded_by: row.try_get("superseded_by").map_err(backend)?,
    })
}

const MEMORY_COLUMNS: &str = "id, user_id, kind, lifecycle, content, importance, confidence,
    source_kind, source_ref, expires_at, created_at, updated_at,
    last_accessed_at, access_count, archived_at, superseded_by";

#[async_trait]
impl MemoryStore for PostgresMemoryStore {
    async fn create(&self, memory: NewMemory) -> Result<Memory, MemoryError> {
        memory.validate()?;

        // The `users` row is the foreign-key anchor; a caller whose principal
        // has never touched this database still needs one. Same pattern as
        // `PostgresConversationStore::ensure`.
        let mut tx = self.pool.begin().await.map_err(backend)?;

        sqlx::query("insert into users (id) values ($1) on conflict (id) do nothing")
            .bind(memory.user_id)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;

        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "insert into memories (
                 user_id, kind, content, importance, confidence,
                 source_kind, source_ref, expires_at
             ) values ($1, $2, $3, $4, $5, $6, $7, $8)
             returning {MEMORY_COLUMNS}"
        )))
        .bind(memory.user_id)
        .bind(memory.kind.as_str())
        .bind(&memory.content)
        .bind(memory.importance.get() as i16)
        .bind(memory.confidence.get())
        .bind(memory.provenance.source_kind.as_str())
        .bind(memory.provenance.source_ref.as_deref())
        .bind(memory.expires_at)
        .fetch_one(&mut *tx)
        .await
        .map_err(backend)?;

        let stored = memory_from_row(&row)?;
        tx.commit().await.map_err(backend)?;
        Ok(stored)
    }

    async fn get(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {MEMORY_COLUMNS} from memories where id = $1 and user_id = $2"
        )))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        match row {
            Some(row) => memory_from_row(&row),
            None => Err(MemoryError::NotFound(id)),
        }
    }

    async fn search(&self, query: MemoryQuery) -> Result<Vec<Memory>, MemoryError> {
        let limit = query.effective_limit() as i64;

        // Every filter is a bind parameter. Empty-list filters degenerate to
        // "any" by passing `NULL`, matched with `is null or ... = any(...)`.
        // Text search uses `to_tsquery` when the query is non-empty; falling
        // back to `ilike` for a single short token is not worth the branch,
        // and `websearch_to_tsquery` accepts single tokens cleanly.
        let lifecycles: Option<Vec<String>> = if query.lifecycles.is_empty() {
            Some(vec![Lifecycle::Active.as_str().to_string()])
        } else {
            Some(
                query
                    .lifecycles
                    .iter()
                    .map(|l| l.as_str().to_string())
                    .collect(),
            )
        };
        let kinds: Option<Vec<String>> = if query.kinds.is_empty() {
            None
        } else {
            Some(query.kinds.iter().map(|k| k.as_str().to_string()).collect())
        };
        let min_importance: Option<i16> = query.min_importance.map(|i| i.get() as i16);
        let text: Option<String> = query
            .text
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());

        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "select {MEMORY_COLUMNS}
             from memories
             where user_id = $1
               and lifecycle = any($2::text[])
               and ($3::text[] is null or kind = any($3::text[]))
               and ($4::smallint is null or importance >= $4::smallint)
               and (
                   $5::text is null
                   or search_tsv @@ websearch_to_tsquery('pg_catalog.english', $5::text)
                   or content ilike '%' || $5::text || '%'
               )
             order by
               case when $5::text is null then 0
                    else ts_rank(search_tsv, websearch_to_tsquery('pg_catalog.english', $5::text))
               end desc,
               importance desc,
               updated_at desc
             limit $6"
        )))
        .bind(query.user_id)
        .bind(lifecycles)
        .bind(kinds)
        .bind(min_importance)
        .bind(text)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        rows.iter().map(memory_from_row).collect()
    }

    async fn update(
        &self,
        user_id: UserId,
        id: MemoryId,
        patch: MemoryPatch,
    ) -> Result<Memory, MemoryError> {
        if patch.is_empty() {
            return self.get(user_id, id).await;
        }
        if let Some(ref content) = patch.content {
            if content.trim().is_empty() {
                return Err(MemoryError::Invalid("memory content is empty".into()));
            }
            if assistant_memory::looks_like_secret(content) {
                return Err(MemoryError::SecretLike);
            }
        }

        // `expires_at` uses double_option semantics: `Some(None)` clears,
        // `Some(Some(_))` sets, `None` leaves alone.
        let expires_clear: bool = matches!(patch.expires_at, Some(None));
        let expires_set: Option<OffsetDateTime> = patch.expires_at.flatten();

        // One statement: `coalesce($n, column)` is the "leave alone" trick used
        // elsewhere in this codebase (see `routes/productivity.rs`). The
        // check constraints handle the temporary/expiry invariant, so a bad
        // combination surfaces as a bad-request rather than a stale read.
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "update memories set
                 kind        = coalesce($1, kind),
                 content     = coalesce($2, content),
                 importance  = coalesce($3::smallint, importance),
                 confidence  = coalesce($4::real, confidence),
                 expires_at  = case
                                   when $5::bool then null
                                   when $6::timestamptz is not null then $6
                                   else expires_at
                               end
             where id = $7 and user_id = $8
             returning {MEMORY_COLUMNS}"
        )))
        .bind(patch.kind.map(|k| k.as_str().to_string()))
        .bind(patch.content.as_ref())
        .bind(patch.importance.map(|i| i.get() as i16))
        .bind(patch.confidence.map(|c| c.get()))
        .bind(expires_clear)
        .bind(expires_set)
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_update_error)?;

        match row {
            Some(row) => memory_from_row(&row),
            None => Err(MemoryError::NotFound(id)),
        }
    }

    async fn archive(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "update memories set
                 lifecycle   = 'archived',
                 archived_at = now()
             where id = $1 and user_id = $2 and lifecycle <> 'superseded'
             returning {MEMORY_COLUMNS}"
        )))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        match row {
            Some(row) => memory_from_row(&row),
            None => Err(MemoryError::NotFound(id)),
        }
    }

    async fn restore(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError> {
        // Restoring a superseded row breaks the invariant it points at, so it
        // is refused here rather than left to a constraint failure.
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "update memories set
                 lifecycle    = 'active',
                 archived_at  = null,
                 superseded_by = null
             where id = $1 and user_id = $2 and lifecycle = 'archived'
             returning {MEMORY_COLUMNS}"
        )))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;

        match row {
            Some(row) => memory_from_row(&row),
            None => Err(MemoryError::NotFound(id)),
        }
    }

    async fn supersede(
        &self,
        user_id: UserId,
        old_id: MemoryId,
        new_id: MemoryId,
    ) -> Result<(), MemoryError> {
        if old_id == new_id {
            return Err(MemoryError::Invalid(
                "a memory cannot supersede itself".into(),
            ));
        }
        let mut tx = self.pool.begin().await.map_err(backend)?;

        // Confirm the new memory belongs to the same principal before pointing
        // the old one at it -- a caller must never make one user's memory
        // "superseded by" another user's row.
        let new_ok: Option<(uuid::Uuid,)> =
            sqlx::query_as("select id from memories where id = $1 and user_id = $2")
                .bind(new_id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(backend)?;
        if new_ok.is_none() {
            return Err(MemoryError::NotFound(new_id));
        }

        let affected = sqlx::query(
            "update memories set
                 lifecycle     = 'superseded',
                 superseded_by = $1,
                 archived_at   = now()
             where id = $2 and user_id = $3 and lifecycle <> 'superseded'",
        )
        .bind(new_id)
        .bind(old_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(backend)?;

        if affected.rows_affected() == 0 {
            return Err(MemoryError::NotFound(old_id));
        }
        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    async fn touch(
        &self,
        user_id: UserId,
        ids: &[MemoryId],
        at: OffsetDateTime,
    ) -> Result<(), MemoryError> {
        if ids.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "update memories set
                 last_accessed_at = $1,
                 access_count     = access_count + 1
             where user_id = $2 and id = any($3::uuid[])",
        )
        .bind(at)
        .bind(user_id)
        .bind(ids)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn sweep_expired(
        &self,
        user_id: UserId,
        now: OffsetDateTime,
    ) -> Result<u64, MemoryError> {
        let result = sqlx::query(
            "update memories set
                 lifecycle   = 'archived',
                 archived_at = $2
             where user_id = $1
               and kind = 'temporary'
               and lifecycle = 'active'
               and expires_at is not null
               and expires_at <= $2",
        )
        .bind(user_id)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(result.rows_affected())
    }
}

/// A constraint hit on `update` is a bad request from the caller (e.g. a
/// `kind` switch that would violate the temporary/expiry invariant), not a
/// generic backend failure.
fn map_update_error(error: sqlx::Error) -> MemoryError {
    if let sqlx::Error::Database(ref db) = error
        && (db.is_check_violation() || db.is_foreign_key_violation() || db.is_unique_violation())
    {
        return MemoryError::Invalid(
            db.constraint()
                .map(|c| format!("value rejected by {c}"))
                .unwrap_or_else(|| "value rejected by a database constraint".to_string()),
        );
    }
    backend(error)
}
