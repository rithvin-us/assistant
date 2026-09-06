//! Postgres access.
//!
//! Supabase is the source of truth. The pool is optional at startup: a missing
//! `DATABASE_URL` degrades the service rather than preventing it from booting,
//! which keeps frontend work unblocked. Migrations are plain SQL under
//! `migrations/` and are applied by `scripts/migrate.ps1`, never implicitly on
//! boot. See docs/DECISIONS.md ADR-0006.

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("could not connect to the database: {0}")]
    Connect(#[source] sqlx::Error),
}

/// Opens a connection pool and verifies it with a round-trip. Connecting lazily
/// would hide a bad `DATABASE_URL` until the first request, which is worse.
pub async fn connect(database_url: &str) -> Result<PgPool, DbError> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .map_err(DbError::Connect)?;

    sqlx::query("select 1")
        .execute(&pool)
        .await
        .map_err(DbError::Connect)?;

    Ok(pool)
}
