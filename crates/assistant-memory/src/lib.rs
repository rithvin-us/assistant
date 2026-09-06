//! Memory domain.
//!
//! Memory is deliberately *not* conversation history. A conversation is a log;
//! a memory is a durable, attributed claim that survived a promotion step. The
//! types here encode that separation so no later code can accidentally treat
//! every message as permanent.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub type MemoryId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// How the user wants things done.
    Preference,
    /// A stable claim about the world or the user.
    Fact,
    /// Something the user floated but has not committed to.
    Idea,
    /// Something the user said they would do.
    Commitment,
    /// Scoped to a project; archived with it.
    Project,
    /// Expires on its own; never promoted without a reason.
    TemporaryContext,
}

/// Where a memory sits in its life. Archived memories stay queryable so the user
/// can recover them; deletion is a separate, explicit act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    /// Suggested by the assistant, not yet accepted.
    Proposed,
    /// Accepted and eligible for retrieval.
    Active,
    /// Hidden from the main view, still recoverable.
    Archived,
}

/// Why a memory exists. Provenance is mandatory: a memory with no traceable
/// source cannot be audited by the user and should not influence answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    /// e.g. `conversation`, `document`, `email`, `manual_capture`.
    pub source_kind: String,
    /// Identifier within that source, e.g. a message or document id.
    pub source_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub kind: MemoryKind,
    pub lifecycle: Lifecycle,
    pub content: String,
    /// How much this should influence retrieval and what the Memory UI shows.
    /// The main view is filtered on this; low-importance memories exist but are
    /// not surfaced by default.
    pub importance: Importance,
    /// How sure the system is that the content is true, `0.0..=1.0`.
    pub confidence: f32,
    pub provenance: Provenance,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_accessed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Importance {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryQuery {
    pub kinds: Vec<MemoryKind>,
    pub lifecycle: Option<Lifecycle>,
    pub min_importance: Option<Importance>,
    pub text: Option<String>,
    pub limit: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("memory {0} not found")]
    NotFound(MemoryId),
    #[error("memory store failure: {0}")]
    Backend(String),
}

/// Storage seam. The Postgres/pgvector implementation arrives with the memory
/// milestone; keeping it behind a trait means the orchestrator can be written
/// and tested against an in-memory fake first.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn propose(&self, memory: Memory) -> Result<MemoryId, MemoryError>;
    async fn get(&self, id: MemoryId) -> Result<Memory, MemoryError>;
    async fn search(&self, query: MemoryQuery) -> Result<Vec<Memory>, MemoryError>;
    async fn set_lifecycle(&self, id: MemoryId, lifecycle: Lifecycle) -> Result<(), MemoryError>;
}
