//! Local SQLite cache.
//!
//! This is a cache and an offline capture buffer, not a second copy of the
//! cloud database. Postgres remains the source of truth; anything here can be
//! deleted without data loss once it has synced. See docs/DECISIONS.md ADR-0007.

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum LocalDbError {
    #[error("could not open the local database: {0}")]
    Open(#[source] sqlx::Error),
    #[error("could not prepare the local schema: {0}")]
    Schema(#[source] sqlx::Error),
}

/// Opens (creating if needed) the local database and applies its schema.
///
/// The schema is applied inline rather than through `sqlx::migrate!` because the
/// local store is disposable: if its shape ever changes incompatibly the correct
/// recovery is to delete the file and re-sync, not to migrate it.
pub async fn open(path: &Path) -> Result<SqlitePool, LocalDbError> {
    let url = format!("sqlite://{}?mode=rwc", path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .map_err(LocalDbError::Open)?;

    sqlx::query(
        r#"
        create table if not exists pending_capture (
            id          text primary key,
            body        text not null,
            created_at  text not null,
            synced_at   text
        );
        "#,
    )
    .execute(&pool)
    .await
    .map_err(LocalDbError::Schema)?;

    Ok(pool)
}
