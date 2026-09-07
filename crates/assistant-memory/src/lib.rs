//! Memory domain.
//!
//! Memory is deliberately *not* conversation history. A conversation is a log;
//! a memory is a durable, attributed claim that survived a promotion step.
//!
//! The core rule the types below encode: **the model proposes, the application
//! stores**. A [`MemoryProposal`] is what a model or an integration can hand
//! over; a [`Memory`] is what the application decided to keep. The two are
//! deliberately distinct types so no code path can insert a proposal directly
//! into storage. Confidence, importance, provenance and lifecycle are all
//! decided by deterministic Rust; nothing here calls out to a model.
//!
//! Storage is a trait. The Postgres implementation lives with the pool in
//! `assistant-server`; the core depends only on the trait. An in-process fake
//! ([`InMemoryMemoryStore`]) exists so orchestration tests can exercise the
//! retrieval path without a database.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use uuid::Uuid;

pub type MemoryId = Uuid;
pub type UserId = Uuid;

// ---------------------------------------------------------------------------
// Core enums
// ---------------------------------------------------------------------------

/// What kind of thing this memory is.
///
/// Kept small on purpose. Adding a variant is a migration (the database check
/// constraint lists these strings), not a free-form label a caller can invent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Scoped to a project; archived when the project is.
    Project,
    /// Expires on its own; never promoted without a reason.
    Temporary,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Idea => "idea",
            Self::Commitment => "commitment",
            Self::Project => "project",
            Self::Temporary => "temporary",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "preference" => Some(Self::Preference),
            "fact" => Some(Self::Fact),
            "idea" => Some(Self::Idea),
            "commitment" => Some(Self::Commitment),
            "project" => Some(Self::Project),
            "temporary" => Some(Self::Temporary),
            _ => None,
        }
    }

    /// Whether a memory of this kind must carry an `expires_at`.
    pub fn requires_expiry(self) -> bool {
        matches!(self, Self::Temporary)
    }
}

/// Where a memory sits in its life.
///
/// Archived rows stay queryable so the user can restore them; superseded rows
/// point at the memory that replaced them so the UI can explain what changed.
/// Destructive deletion is a separate, explicit act -- never the default UI
/// action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Active,
    Archived,
    Superseded,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "archived" => Some(Self::Archived),
            "superseded" => Some(Self::Superseded),
            _ => None,
        }
    }
}

/// Where a memory came from.
///
/// Mandatory on every memory: a memory with no traceable source cannot be
/// audited by the user and should not influence answers. Bounded so a
/// retrieval that filters by provenance cannot silently miss a source that
/// named itself differently one week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// The user asked, in so many words, to remember this.
    ExplicitUserInput,
    /// Extracted from a conversation.
    Conversation,
    /// A task row.
    Task,
    /// A note row.
    Note,
    /// An idea row.
    Idea,
    /// A project row.
    Project,
    /// A document (Drive file, PDF, etc.).
    Document,
    /// Anything from a third-party integration not covered above.
    ExternalSource,
}

impl MemorySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitUserInput => "explicit_user_input",
            Self::Conversation => "conversation",
            Self::Task => "task",
            Self::Note => "note",
            Self::Idea => "idea",
            Self::Project => "project",
            Self::Document => "document",
            Self::ExternalSource => "external_source",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "explicit_user_input" => Some(Self::ExplicitUserInput),
            "conversation" => Some(Self::Conversation),
            "task" => Some(Self::Task),
            "note" => Some(Self::Note),
            "idea" => Some(Self::Idea),
            "project" => Some(Self::Project),
            "document" => Some(Self::Document),
            "external_source" => Some(Self::ExternalSource),
            _ => None,
        }
    }
}

/// Provenance tuple.
///
/// `source_ref` is free-form because not every source is a row in this
/// database; for a Classroom announcement it might be a provider id, and for
/// an explicit user request it is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_kind: MemorySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

impl Provenance {
    pub fn explicit() -> Self {
        Self {
            source_kind: MemorySource::ExplicitUserInput,
            source_ref: None,
        }
    }

    pub fn from_source(kind: MemorySource, source_ref: impl Into<String>) -> Self {
        Self {
            source_kind: kind,
            source_ref: Some(source_ref.into()),
        }
    }
}

/// A 1..=5 deterministic priority. `3` is the default "normal".
///
/// Integer scale rather than a fuzzy Low/Normal/High enum because the UI wants
/// to sort by it and the ranking layer wants to weight by it; both are cleaner
/// against an integer than against enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Importance(u8);

impl Importance {
    pub const MIN: Importance = Importance(1);
    pub const LOW: Importance = Importance(2);
    pub const NORMAL: Importance = Importance(3);
    pub const HIGH: Importance = Importance(4);
    pub const MAX: Importance = Importance(5);

    pub fn new(value: u8) -> Result<Self, MemoryError> {
        if (1..=5).contains(&value) {
            Ok(Self(value))
        } else {
            Err(MemoryError::Invalid(format!(
                "importance {value} is outside 1..=5"
            )))
        }
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for Importance {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Confidence in the memory's content, in `0.0..=1.0`.
///
/// Kept separate from [`Importance`]: importance says how much the user cares,
/// confidence says how sure the system is the content is true.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(f32);

impl Confidence {
    pub const CERTAIN: Confidence = Confidence(1.0);
    pub const DEFAULT: Confidence = Confidence(0.7);

    pub fn new(value: f32) -> Result<Self, MemoryError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(MemoryError::Invalid(format!(
                "confidence {value} is outside 0.0..=1.0"
            )))
        }
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// ---------------------------------------------------------------------------
// Domain records
// ---------------------------------------------------------------------------

/// A memory the application has decided to keep.
///
/// Every field is authoritative. The provenance is mandatory, the lifecycle is
/// a bounded enum, the timestamps come from the database, and nothing here is
/// ever written from raw model output -- writes go through [`NewMemory`],
/// which is constructed by application code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub user_id: UserId,
    pub kind: MemoryKind,
    pub lifecycle: Lifecycle,
    pub content: String,
    pub importance: Importance,
    pub confidence: Confidence,
    pub provenance: Provenance,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_accessed_at: Option<OffsetDateTime>,
    pub access_count: u32,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub archived_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<MemoryId>,
}

/// Input shape for [`MemoryStore::create`].
///
/// Constructed by application code from a validated proposal, an explicit
/// user request, or an integration import. The store never invents a value
/// that is not here.
#[derive(Debug, Clone)]
pub struct NewMemory {
    pub user_id: UserId,
    pub kind: MemoryKind,
    pub content: String,
    pub importance: Importance,
    pub confidence: Confidence,
    pub provenance: Provenance,
    /// Required when `kind == Temporary`, forbidden otherwise. Validated by
    /// the store's create path.
    pub expires_at: Option<OffsetDateTime>,
}

impl NewMemory {
    /// Convenience for the explicit-user-input path.
    pub fn explicit(user_id: UserId, kind: MemoryKind, content: impl Into<String>) -> Self {
        Self {
            user_id,
            kind,
            content: content.into(),
            importance: Importance::NORMAL,
            confidence: Confidence::CERTAIN,
            provenance: Provenance::explicit(),
            expires_at: None,
        }
    }

    pub fn with_importance(mut self, importance: Importance) -> Self {
        self.importance = importance;
        self
    }

    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = confidence;
        self
    }

    pub fn with_expiry(mut self, expires_at: OffsetDateTime) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Cheap validation before the store round-trips.
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.content.trim().is_empty() {
            return Err(MemoryError::Invalid("memory content is empty".into()));
        }
        if self.content.len() > MAX_CONTENT_BYTES {
            return Err(MemoryError::Invalid(format!(
                "memory content is longer than {MAX_CONTENT_BYTES} bytes"
            )));
        }
        if self.kind.requires_expiry() && self.expires_at.is_none() {
            return Err(MemoryError::Invalid(
                "temporary memories require an expires_at".into(),
            ));
        }
        if !self.kind.requires_expiry() && self.expires_at.is_some() {
            return Err(MemoryError::Invalid(
                "expires_at is only valid on temporary memories".into(),
            ));
        }
        if looks_like_secret(&self.content) {
            return Err(MemoryError::SecretLike);
        }
        Ok(())
    }
}

/// A patch for [`MemoryStore::update`].
///
/// Every field is optional; a `None` means "leave alone". A patch cannot
/// change the owner or the provenance -- both are load-bearing for audit and
/// authorisation, so mutating them would defeat their purpose.
#[derive(Debug, Clone, Default)]
pub struct MemoryPatch {
    pub kind: Option<MemoryKind>,
    pub content: Option<String>,
    pub importance: Option<Importance>,
    pub confidence: Option<Confidence>,
    /// `Some(Some(_))` sets, `Some(None)` clears, `None` leaves alone. Only
    /// valid for temporary memories; the store re-checks this.
    pub expires_at: Option<Option<OffsetDateTime>>,
}

impl MemoryPatch {
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.content.is_none()
            && self.importance.is_none()
            && self.confidence.is_none()
            && self.expires_at.is_none()
    }
}

/// A model-independent suggestion.
///
/// A model or an integration returns one of these; the application then
/// decides whether to keep it. `MemoryProposal` never touches durable storage
/// on its own -- the boundary is the [`MemoryProposal::accept`] step, which
/// runs deterministic checks and, on success, hands back a [`NewMemory`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProposal {
    pub kind: MemoryKind,
    pub content: String,
    /// The proposer's own confidence. Used as a suggestion; the application
    /// may reject or downgrade it.
    #[serde(default)]
    pub confidence: Option<f32>,
    /// The proposer's own importance suggestion, on the 1..=5 scale.
    #[serde(default)]
    pub importance: Option<u8>,
    /// The proposer's own reason. Recorded for audit; never used as the
    /// memory's content.
    #[serde(default)]
    pub reason: Option<String>,
    pub source_kind: MemorySource,
    #[serde(default)]
    pub source_ref: Option<String>,
    /// Required for `kind == Temporary`.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
}

impl MemoryProposal {
    /// Validates the proposal and, on success, produces a [`NewMemory`] the
    /// caller can hand to [`MemoryStore::create`].
    ///
    /// The importance is clamped to `1..=5`; the confidence is clamped to
    /// `[0, 1]`; obviously secret-looking content is refused. This is where
    /// "the application decides" happens, and it happens in one place.
    pub fn accept(self, user_id: UserId) -> Result<NewMemory, MemoryError> {
        let confidence = match self.confidence {
            Some(value) if value.is_finite() => Confidence::new(value.clamp(0.0, 1.0))?,
            _ => Confidence::DEFAULT,
        };
        let importance = match self.importance {
            Some(value) => Importance::new(value.clamp(1, 5))?,
            None => Importance::NORMAL,
        };
        let provenance = Provenance {
            source_kind: self.source_kind,
            source_ref: self.source_ref,
        };
        let candidate = NewMemory {
            user_id,
            kind: self.kind,
            content: self.content,
            importance,
            confidence,
            provenance,
            expires_at: self.expires_at,
        };
        candidate.validate()?;
        Ok(candidate)
    }
}

// ---------------------------------------------------------------------------
// Retrieval
// ---------------------------------------------------------------------------

/// Filter for [`MemoryStore::search`].
#[derive(Debug, Clone, Default)]
pub struct MemoryQuery {
    pub user_id: UserId,
    /// If empty, all kinds match.
    pub kinds: Vec<MemoryKind>,
    /// If empty, only `Active` matches. The archive UI passes explicit values.
    pub lifecycles: Vec<Lifecycle>,
    pub min_importance: Option<Importance>,
    /// Free-form text. The store runs a text-search predicate against it.
    pub text: Option<String>,
    /// Hard cap on the result count. The store enforces its own upper bound if
    /// this is zero or absurdly large.
    pub limit: u32,
}

impl MemoryQuery {
    pub fn active(user_id: UserId) -> Self {
        Self {
            user_id,
            lifecycles: vec![Lifecycle::Active],
            limit: DEFAULT_SEARCH_LIMIT,
            ..Default::default()
        }
    }

    pub fn effective_limit(&self) -> u32 {
        let raw = if self.limit == 0 {
            DEFAULT_SEARCH_LIMIT
        } else {
            self.limit
        };
        raw.min(MAX_SEARCH_LIMIT)
    }
}

/// One row from a ranked retrieval.
#[derive(Debug, Clone)]
pub struct ScoredMemory {
    pub memory: Memory,
    /// Non-negative. Larger is more relevant. Deterministic combination of
    /// text hit, importance, confidence, recency and access count -- see
    /// [`rank`].
    pub score: f32,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("memory {0} not found")]
    NotFound(MemoryId),
    #[error("invalid input: {0}")]
    Invalid(String),
    /// The content looked like a credential. Refused before it hit storage.
    #[error("content looks like a credential; memory not stored")]
    SecretLike,
    #[error("memory store failure: {0}")]
    Backend(String),
}

impl MemoryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::Invalid(_) => "invalid",
            Self::SecretLike => "secret_like",
            Self::Backend(_) => "backend",
        }
    }
}

// ---------------------------------------------------------------------------
// Storage seam
// ---------------------------------------------------------------------------

/// The durable memory store.
///
/// Every method takes the authenticated user's id and scopes its work by it.
/// An implementation must never trust a caller-supplied `user_id` -- the
/// signature is here so a caller cannot forget to pass one.
///
/// The `sqlx` implementation lives in `assistant-server`; the core depends
/// only on this trait.
#[async_trait]
pub trait MemoryStore: Send + Sync {
    async fn create(&self, memory: NewMemory) -> Result<Memory, MemoryError>;

    async fn get(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError>;

    /// Bounded list; the store enforces its own upper bound. Never returns
    /// the whole memory set.
    async fn search(&self, query: MemoryQuery) -> Result<Vec<Memory>, MemoryError>;

    async fn update(
        &self,
        user_id: UserId,
        id: MemoryId,
        patch: MemoryPatch,
    ) -> Result<Memory, MemoryError>;

    async fn archive(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError>;

    async fn restore(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError>;

    /// Marks `old_id` as superseded by `new_id`; both must belong to the same
    /// user. Called by the application when the caller decides that a new
    /// memory replaces an existing one.
    async fn supersede(
        &self,
        user_id: UserId,
        old_id: MemoryId,
        new_id: MemoryId,
    ) -> Result<(), MemoryError>;

    /// Bumps `last_accessed_at` to `at` and increments `access_count`. Called
    /// by the retrieval layer *only* when the memory was actually used to
    /// answer a turn -- not for every incidental read.
    async fn touch(
        &self,
        user_id: UserId,
        ids: &[MemoryId],
        at: OffsetDateTime,
    ) -> Result<(), MemoryError>;

    /// Moves expired temporary memories into `archived`. Returns how many
    /// were moved. Called on the retrieval path so nothing needs a scheduler.
    async fn sweep_expired(&self, user_id: UserId, now: OffsetDateTime)
    -> Result<u64, MemoryError>;
}

// ---------------------------------------------------------------------------
// Constants and helpers
// ---------------------------------------------------------------------------

/// Hard upper bound on `content` length. Chosen so a runaway extractor cannot
/// blow the row past sensible size while still comfortably fitting a
/// paragraph-long preference or fact.
pub const MAX_CONTENT_BYTES: usize = 4096;

/// What the API returns when a caller does not name a size.
pub const DEFAULT_SEARCH_LIMIT: u32 = 25;

/// What the API returns even when a caller asks for more. The context path
/// asks for far less than this; this is the wall against pathological asks.
pub const MAX_SEARCH_LIMIT: u32 = 200;

/// Bound on how many memories may be injected into one turn's context.
///
/// Kept small on purpose: memory should influence the model, not drown it, and
/// a bounded context is a testable one.
pub const MAX_CONTEXT_MEMORIES: usize = 8;

/// Heuristic secret detector for the explicit-memory path.
///
/// Not a security control -- a determined user can still write a password
/// into a memory -- but it catches the obvious cases so the assistant does
/// not silently persist an API key someone pasted into a chat asking it to
/// "remember this".
pub fn looks_like_secret(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    // Common credential prefixes and key-shaped tokens.
    const NEEDLES: &[&str] = &[
        "api_key=",
        "api-key=",
        "apikey=",
        "secret=",
        "password=",
        "passwd=",
        "authorization: bearer",
        "bearer ey",
        "-----begin ",
        "sk-",
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "github_pat_",
        "aws_secret",
        "xoxb-",
        "xoxp-",
    ];
    if NEEDLES.iter().any(|needle| lower.contains(needle)) {
        return true;
    }
    // Long unbroken high-entropy blob: a run of >=32 alphanumeric chars
    // between separators, containing both letters and digits. UUIDs (max
    // segment length 12) and human-readable slugs are broken by hyphens and
    // dots so they do not trip this. Base64 payloads (JWT parts, github PATs
    // after their prefix, generic API keys) do.
    for token in content.split(|c: char| !c.is_ascii_alphanumeric()) {
        if token.len() >= 32
            && token.chars().any(|c| c.is_ascii_digit())
            && token.chars().any(|c| c.is_ascii_alphabetic())
        {
            return true;
        }
    }
    false
}

/// Weights for [`rank`].
///
/// Deterministic and named so an operator can reason about the ordering
/// without reading the algorithm. Adjusting these does not need a new
/// milestone; adjusting them silently would.
#[derive(Debug, Clone, Copy)]
pub struct RankingWeights {
    pub text: f32,
    pub importance: f32,
    pub confidence: f32,
    pub recency: f32,
    pub usage: f32,
}

impl Default for RankingWeights {
    fn default() -> Self {
        Self {
            text: 3.0,
            importance: 1.0,
            confidence: 0.5,
            recency: 0.5,
            usage: 0.25,
        }
    }
}

/// Ranks a set of memories against a query, deterministically.
///
/// The score combines five signals: naive text overlap, importance, confidence,
/// recency (24h decay) and prior usage. Nothing here is ML: the intent is a
/// baseline that is transparent and testable, and can later evolve.
pub fn rank(memories: &[Memory], query: Option<&str>, now: OffsetDateTime) -> Vec<ScoredMemory> {
    let weights = RankingWeights::default();
    let query_tokens: Vec<String> = query
        .map(|text| {
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|token| token.len() >= 2)
                .map(|token| token.to_string())
                .collect()
        })
        .unwrap_or_default();

    let mut scored: Vec<ScoredMemory> = memories
        .iter()
        .map(|memory| {
            let mut score = 0.0f32;

            if !query_tokens.is_empty() {
                let content = memory.content.to_lowercase();
                let hits = query_tokens
                    .iter()
                    .filter(|token| content.contains(token.as_str()))
                    .count();
                if hits > 0 {
                    // Log-ish saturation: a memory that hits every token is
                    // rewarded but not exponentially so.
                    score += weights.text
                        * (hits as f32 / query_tokens.len() as f32)
                        * (1.0 + (hits as f32).ln_1p());
                }
            }

            score += weights.importance * ((memory.importance.get() as f32 - 1.0) / 4.0);
            score += weights.confidence * memory.confidence.get();

            let age_hours = (now - memory.updated_at).whole_seconds().max(0) as f32 / 3600.0;
            let recency_decay = (-age_hours / 24.0).exp();
            score += weights.recency * recency_decay;

            let usage = (memory.access_count as f32 + 1.0).ln();
            score += weights.usage * usage;

            ScoredMemory {
                memory: memory.clone(),
                score,
            }
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

// ---------------------------------------------------------------------------
// In-memory store (for tests and non-durable deployments)
// ---------------------------------------------------------------------------

/// Non-durable [`MemoryStore`] for tests and for the deployment with no
/// database. Not intended for production use; state is lost on restart.
#[derive(Default)]
pub struct InMemoryMemoryStore {
    rows: RwLock<HashMap<MemoryId, Memory>>,
}

impl InMemoryMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MemoryStore for InMemoryMemoryStore {
    async fn create(&self, memory: NewMemory) -> Result<Memory, MemoryError> {
        memory.validate()?;
        let now = OffsetDateTime::now_utc();
        let stored = Memory {
            id: Uuid::new_v4(),
            user_id: memory.user_id,
            kind: memory.kind,
            lifecycle: Lifecycle::Active,
            content: memory.content,
            importance: memory.importance,
            confidence: memory.confidence,
            provenance: memory.provenance,
            expires_at: memory.expires_at,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
            archived_at: None,
            superseded_by: None,
        };
        self.rows.write().await.insert(stored.id, stored.clone());
        Ok(stored)
    }

    async fn get(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError> {
        let rows = self.rows.read().await;
        rows.get(&id)
            .filter(|row| row.user_id == user_id)
            .cloned()
            .ok_or(MemoryError::NotFound(id))
    }

    async fn search(&self, query: MemoryQuery) -> Result<Vec<Memory>, MemoryError> {
        let rows = self.rows.read().await;
        let limit = query.effective_limit() as usize;
        let text = query.text.as_deref().map(str::to_lowercase);
        let lifecycles: Vec<Lifecycle> = if query.lifecycles.is_empty() {
            vec![Lifecycle::Active]
        } else {
            query.lifecycles.clone()
        };
        let mut matches: Vec<Memory> = rows
            .values()
            .filter(|row| row.user_id == query.user_id)
            .filter(|row| lifecycles.contains(&row.lifecycle))
            .filter(|row| query.kinds.is_empty() || query.kinds.contains(&row.kind))
            .filter(|row| query.min_importance.is_none_or(|min| row.importance >= min))
            .filter(|row| {
                text.as_ref()
                    .is_none_or(|t| row.content.to_lowercase().contains(t))
            })
            .cloned()
            .collect();
        matches.sort_by_key(|memory| std::cmp::Reverse(memory.updated_at));
        matches.truncate(limit);
        Ok(matches)
    }

    async fn update(
        &self,
        user_id: UserId,
        id: MemoryId,
        patch: MemoryPatch,
    ) -> Result<Memory, MemoryError> {
        let mut rows = self.rows.write().await;
        let row = rows
            .get_mut(&id)
            .filter(|row| row.user_id == user_id)
            .ok_or(MemoryError::NotFound(id))?;
        if let Some(kind) = patch.kind {
            row.kind = kind;
        }
        if let Some(content) = patch.content {
            if content.trim().is_empty() {
                return Err(MemoryError::Invalid("memory content is empty".into()));
            }
            if looks_like_secret(&content) {
                return Err(MemoryError::SecretLike);
            }
            row.content = content;
        }
        if let Some(importance) = patch.importance {
            row.importance = importance;
        }
        if let Some(confidence) = patch.confidence {
            row.confidence = confidence;
        }
        if let Some(expires_at) = patch.expires_at {
            row.expires_at = expires_at;
        }
        if row.kind.requires_expiry() && row.expires_at.is_none() {
            return Err(MemoryError::Invalid(
                "temporary memories require an expires_at".into(),
            ));
        }
        if !row.kind.requires_expiry() && row.expires_at.is_some() {
            return Err(MemoryError::Invalid(
                "expires_at is only valid on temporary memories".into(),
            ));
        }
        row.updated_at = OffsetDateTime::now_utc();
        Ok(row.clone())
    }

    async fn archive(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError> {
        let mut rows = self.rows.write().await;
        let row = rows
            .get_mut(&id)
            .filter(|row| row.user_id == user_id)
            .ok_or(MemoryError::NotFound(id))?;
        let now = OffsetDateTime::now_utc();
        row.lifecycle = Lifecycle::Archived;
        row.archived_at = Some(now);
        row.updated_at = now;
        Ok(row.clone())
    }

    async fn restore(&self, user_id: UserId, id: MemoryId) -> Result<Memory, MemoryError> {
        let mut rows = self.rows.write().await;
        let row = rows
            .get_mut(&id)
            .filter(|row| row.user_id == user_id)
            .ok_or(MemoryError::NotFound(id))?;
        row.lifecycle = Lifecycle::Active;
        row.archived_at = None;
        row.superseded_by = None;
        row.updated_at = OffsetDateTime::now_utc();
        Ok(row.clone())
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
        let mut rows = self.rows.write().await;
        let new_ok = rows.get(&new_id).is_some_and(|row| row.user_id == user_id);
        if !new_ok {
            return Err(MemoryError::NotFound(new_id));
        }
        let old = rows
            .get_mut(&old_id)
            .filter(|row| row.user_id == user_id)
            .ok_or(MemoryError::NotFound(old_id))?;
        let now = OffsetDateTime::now_utc();
        old.lifecycle = Lifecycle::Superseded;
        old.superseded_by = Some(new_id);
        old.archived_at = Some(now);
        old.updated_at = now;
        Ok(())
    }

    async fn touch(
        &self,
        user_id: UserId,
        ids: &[MemoryId],
        at: OffsetDateTime,
    ) -> Result<(), MemoryError> {
        let mut rows = self.rows.write().await;
        for id in ids {
            if let Some(row) = rows.get_mut(id).filter(|row| row.user_id == user_id) {
                row.last_accessed_at = Some(at);
                row.access_count = row.access_count.saturating_add(1);
            }
        }
        Ok(())
    }

    async fn sweep_expired(
        &self,
        user_id: UserId,
        now: OffsetDateTime,
    ) -> Result<u64, MemoryError> {
        let mut rows = self.rows.write().await;
        let mut moved: u64 = 0;
        for row in rows.values_mut() {
            if row.user_id != user_id
                || row.lifecycle != Lifecycle::Active
                || row.kind != MemoryKind::Temporary
            {
                continue;
            }
            if row.expires_at.is_some_and(|expiry| expiry <= now) {
                row.lifecycle = Lifecycle::Archived;
                row.archived_at = Some(now);
                row.updated_at = now;
                moved += 1;
            }
        }
        Ok(moved)
    }
}

// ---------------------------------------------------------------------------
// Explicit-memory detection
// ---------------------------------------------------------------------------

/// Parsed "remember that ..." instruction.
///
/// Produced by [`parse_explicit_memory`] from a normalised user utterance.
/// The application uses it to insert a memory without invoking any model at
/// all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitMemoryInstruction {
    pub kind: MemoryKind,
    pub content: String,
}

/// Detects an explicit "remember that ..." style instruction.
///
/// Deterministic, no model. Recognises a handful of natural phrasings and
/// leaves everything else alone; the point is that a user can *say* "remember
/// I prefer X" and know exactly what happens next, not that every plausible
/// wording is caught.
pub fn parse_explicit_memory(text: &str) -> Option<ExplicitMemoryInstruction> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_lowercase();

    const PREFIXES: &[&str] = &[
        "remember that ",
        "please remember that ",
        "remember, ",
        "remember: ",
        "remember ",
        "note that ",
        "note to self: ",
        "note: ",
        "keep in mind that ",
        "keep in mind, ",
    ];

    for prefix in PREFIXES {
        if let Some(stripped) = lower.strip_prefix(prefix) {
            // Preserve the user's original casing for the content itself.
            let start = trimmed.len() - stripped.len();
            let body = trimmed[start..].trim().trim_end_matches('.').trim();
            if body.is_empty() {
                return None;
            }
            let kind = infer_kind(body);
            return Some(ExplicitMemoryInstruction {
                kind,
                content: body.to_string(),
            });
        }
    }

    None
}

fn infer_kind(body: &str) -> MemoryKind {
    let lower = body.to_lowercase();
    if lower.starts_with("i prefer ")
        || lower.starts_with("i like ")
        || lower.starts_with("i don't like ")
        || lower.starts_with("i dislike ")
        || lower.contains(" prefer ")
    {
        MemoryKind::Preference
    } else if lower.starts_with("i will ")
        || lower.starts_with("i'll ")
        || lower.starts_with("i am going to ")
        || lower.starts_with("i'm going to ")
    {
        MemoryKind::Commitment
    } else {
        MemoryKind::Fact
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn user() -> UserId {
        Uuid::new_v4()
    }

    #[test]
    fn importance_rejects_out_of_range() {
        assert!(Importance::new(0).is_err());
        assert!(Importance::new(6).is_err());
        assert_eq!(Importance::new(3).unwrap().get(), 3);
    }

    #[test]
    fn confidence_rejects_out_of_range() {
        assert!(Confidence::new(-0.1).is_err());
        assert!(Confidence::new(1.1).is_err());
        assert!(Confidence::new(f32::NAN).is_err());
        assert_eq!(Confidence::new(0.5).unwrap().get(), 0.5);
    }

    #[test]
    fn new_memory_requires_content() {
        let m = NewMemory::explicit(user(), MemoryKind::Fact, "   ");
        assert!(matches!(m.validate(), Err(MemoryError::Invalid(_))));
    }

    #[test]
    fn temporary_requires_expiry_and_others_forbid_it() {
        let m = NewMemory::explicit(user(), MemoryKind::Temporary, "tonight only");
        assert!(matches!(m.validate(), Err(MemoryError::Invalid(_))));

        let now = OffsetDateTime::now_utc();
        let mut ok = NewMemory::explicit(user(), MemoryKind::Temporary, "tonight only");
        ok.expires_at = Some(now + time::Duration::hours(6));
        assert!(ok.validate().is_ok());

        let mut bad = NewMemory::explicit(user(), MemoryKind::Fact, "the sky is blue");
        bad.expires_at = Some(now);
        assert!(matches!(bad.validate(), Err(MemoryError::Invalid(_))));
    }

    #[test]
    fn secret_like_content_is_rejected() {
        assert!(looks_like_secret(
            "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5c"
        ));
        assert!(looks_like_secret("api_key=abcdef1234567890"));
        assert!(looks_like_secret("-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(looks_like_secret("here is my token: sk-abcdefghijklmnop"));
        assert!(!looks_like_secret(
            "I prefer concise technical explanations."
        ));
    }

    #[test]
    fn proposal_accept_produces_a_valid_new_memory() {
        let uid = user();
        let proposal = MemoryProposal {
            kind: MemoryKind::Preference,
            content: "The user prefers Python for scripting.".into(),
            confidence: Some(0.8),
            importance: Some(4),
            reason: Some("mentioned twice in this session".into()),
            source_kind: MemorySource::Conversation,
            source_ref: Some("conv-1".into()),
            expires_at: None,
        };
        let new_memory = proposal.accept(uid).expect("valid");
        assert_eq!(new_memory.user_id, uid);
        assert_eq!(new_memory.importance.get(), 4);
        assert!((new_memory.confidence.get() - 0.8).abs() < 1e-6);
        assert_eq!(
            new_memory.provenance.source_kind,
            MemorySource::Conversation
        );
    }

    #[test]
    fn proposal_clamps_out_of_range_confidence_and_importance() {
        let proposal = MemoryProposal {
            kind: MemoryKind::Fact,
            content: "The Earth orbits the Sun.".into(),
            confidence: Some(2.0),
            importance: Some(99),
            reason: None,
            source_kind: MemorySource::ExternalSource,
            source_ref: None,
            expires_at: None,
        };
        let candidate = proposal.accept(user()).expect("valid after clamp");
        assert!((candidate.confidence.get() - 1.0).abs() < 1e-6);
        assert_eq!(candidate.importance.get(), 5);
    }

    #[test]
    fn proposal_rejects_secret_like_content() {
        let proposal = MemoryProposal {
            kind: MemoryKind::Fact,
            content: "api_key=abcdef1234567890abcdef1234567890".into(),
            confidence: None,
            importance: None,
            reason: None,
            source_kind: MemorySource::Conversation,
            source_ref: None,
            expires_at: None,
        };
        assert!(matches!(
            proposal.accept(user()),
            Err(MemoryError::SecretLike)
        ));
    }

    #[tokio::test]
    async fn in_memory_store_scopes_by_user() {
        let store = InMemoryMemoryStore::new();
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

        // Bob may never read Alice's memory.
        assert!(matches!(
            store.get(bob, alice_memory.id).await,
            Err(MemoryError::NotFound(_))
        ));

        let bob_hits = store
            .search(MemoryQuery::active(bob))
            .await
            .expect("searched");
        assert!(bob_hits.is_empty(), "cross-user leak in search");
    }

    #[tokio::test]
    async fn archive_and_restore_move_lifecycle() {
        let store = InMemoryMemoryStore::new();
        let uid = user();
        let stored = store
            .create(NewMemory::explicit(uid, MemoryKind::Fact, "hello"))
            .await
            .unwrap();
        let archived = store.archive(uid, stored.id).await.unwrap();
        assert_eq!(archived.lifecycle, Lifecycle::Archived);
        assert!(archived.archived_at.is_some());
        let restored = store.restore(uid, stored.id).await.unwrap();
        assert_eq!(restored.lifecycle, Lifecycle::Active);
        assert!(restored.archived_at.is_none());
    }

    #[tokio::test]
    async fn supersede_marks_the_old_row_and_points_at_the_new() {
        let store = InMemoryMemoryStore::new();
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

        let refreshed_old = store.get(uid, old.id).await.unwrap();
        assert_eq!(refreshed_old.lifecycle, Lifecycle::Superseded);
        assert_eq!(refreshed_old.superseded_by, Some(new_memory.id));
    }

    #[tokio::test]
    async fn touch_bumps_access_metadata() {
        let store = InMemoryMemoryStore::new();
        let uid = user();
        let stored = store
            .create(NewMemory::explicit(uid, MemoryKind::Fact, "hi"))
            .await
            .unwrap();
        assert_eq!(stored.access_count, 0);
        assert!(stored.last_accessed_at.is_none());

        let now = OffsetDateTime::now_utc();
        store.touch(uid, &[stored.id], now).await.unwrap();
        let refreshed = store.get(uid, stored.id).await.unwrap();
        assert_eq!(refreshed.access_count, 1);
        assert_eq!(refreshed.last_accessed_at, Some(now));
    }

    #[tokio::test]
    async fn sweep_expires_temporary_memories() {
        let store = InMemoryMemoryStore::new();
        let uid = user();
        let now = OffsetDateTime::now_utc();
        let expiring = NewMemory::explicit(uid, MemoryKind::Temporary, "tonight")
            .with_expiry(now - time::Duration::seconds(1));
        let stored = store.create(expiring).await.unwrap();
        let moved = store.sweep_expired(uid, now).await.unwrap();
        assert_eq!(moved, 1);
        let refreshed = store.get(uid, stored.id).await.unwrap();
        assert_eq!(refreshed.lifecycle, Lifecycle::Archived);
    }

    #[test]
    fn rank_prefers_a_query_hit_over_an_older_high_importance_miss() {
        let uid = user();
        let now = OffsetDateTime::now_utc();
        let base = Memory {
            id: Uuid::new_v4(),
            user_id: uid,
            kind: MemoryKind::Fact,
            lifecycle: Lifecycle::Active,
            content: "prefers cats".into(),
            importance: Importance::LOW,
            confidence: Confidence::CERTAIN,
            provenance: Provenance::explicit(),
            expires_at: None,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
            archived_at: None,
            superseded_by: None,
        };
        let hit = base.clone();
        let mut miss = base.clone();
        miss.id = Uuid::new_v4();
        miss.content = "the sky is blue".into();
        miss.importance = Importance::MAX;
        let scored = rank(&[hit.clone(), miss.clone()], Some("cats"), now);
        assert_eq!(scored.first().unwrap().memory.id, hit.id);
    }

    #[test]
    fn parse_explicit_memory_recognises_common_phrasings() {
        let cases: &[(&str, MemoryKind, &str)] = &[
            (
                "Remember that I prefer concise answers.",
                MemoryKind::Preference,
                "I prefer concise answers",
            ),
            (
                "please remember that my dog's name is Ada",
                MemoryKind::Fact,
                "my dog's name is Ada",
            ),
            (
                "Note to self: I will submit the essay tonight.",
                MemoryKind::Commitment,
                "I will submit the essay tonight",
            ),
        ];
        for (input, kind, content) in cases {
            let parsed = parse_explicit_memory(input).expect("parsed");
            assert_eq!(parsed.kind, *kind, "kind for {input:?}");
            assert_eq!(parsed.content, *content, "content for {input:?}");
        }
        assert!(parse_explicit_memory("what time is it?").is_none());
    }

    #[test]
    fn effective_limit_clamps_to_the_maximum() {
        let q = MemoryQuery {
            user_id: user(),
            limit: 10_000,
            ..Default::default()
        };
        assert_eq!(q.effective_limit(), MAX_SEARCH_LIMIT);

        let q = MemoryQuery::active(user());
        assert_eq!(q.effective_limit(), DEFAULT_SEARCH_LIMIT);
    }
}
