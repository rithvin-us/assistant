//! REST endpoints for long-term memory.
//!
//! Every ownership check happens server-side against the authenticated
//! principal (never a client-supplied `user_id`), and every write is validated
//! against the domain rules in `assistant-memory` before it hits the store.
//!
//! `POST /v1/memories` is the explicit path: it accepts a small typed request
//! and inserts a memory without invoking any model. Optional `supersedes` lets
//! the client (or a model that had a proposal accepted) replace an earlier
//! memory in the same round trip.
//!
//! Search is one endpoint with query parameters -- kind, lifecycle, min
//! importance, text and limit. The store enforces its own upper bound on
//! `limit`, so a client cannot pull the whole memory set with one call.

use std::sync::Arc;

use assistant_auth::Principal;
use assistant_memory::{
    Confidence, Importance, Lifecycle, Memory, MemoryError, MemoryKind, MemoryPatch, MemoryQuery,
    MemorySource, MemoryStore, NewMemory, Provenance,
};
use assistant_protocol::{
    CreateMemoryRequest, MemoryItem, MemoryKindDto, MemoryLifecycleDto, MemoryProvenanceDto,
    MemorySourceDto, UpdateMemoryRequest,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{error::AppError, state::SharedState};

fn memory_store(state: &SharedState) -> Result<&Arc<dyn MemoryStore>, AppError> {
    state
        .memory
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("memory store unavailable")))
}

// ---------------------------------------------------------------------------
// Enum conversions
// ---------------------------------------------------------------------------

fn kind_from(dto: MemoryKindDto) -> MemoryKind {
    match dto {
        MemoryKindDto::Preference => MemoryKind::Preference,
        MemoryKindDto::Fact => MemoryKind::Fact,
        MemoryKindDto::Idea => MemoryKind::Idea,
        MemoryKindDto::Commitment => MemoryKind::Commitment,
        MemoryKindDto::Project => MemoryKind::Project,
        MemoryKindDto::Temporary => MemoryKind::Temporary,
    }
}

fn kind_to(kind: MemoryKind) -> MemoryKindDto {
    match kind {
        MemoryKind::Preference => MemoryKindDto::Preference,
        MemoryKind::Fact => MemoryKindDto::Fact,
        MemoryKind::Idea => MemoryKindDto::Idea,
        MemoryKind::Commitment => MemoryKindDto::Commitment,
        MemoryKind::Project => MemoryKindDto::Project,
        MemoryKind::Temporary => MemoryKindDto::Temporary,
    }
}

fn lifecycle_from(dto: MemoryLifecycleDto) -> Lifecycle {
    match dto {
        MemoryLifecycleDto::Active => Lifecycle::Active,
        MemoryLifecycleDto::Archived => Lifecycle::Archived,
        MemoryLifecycleDto::Superseded => Lifecycle::Superseded,
    }
}

fn lifecycle_to(lifecycle: Lifecycle) -> MemoryLifecycleDto {
    match lifecycle {
        Lifecycle::Active => MemoryLifecycleDto::Active,
        Lifecycle::Archived => MemoryLifecycleDto::Archived,
        Lifecycle::Superseded => MemoryLifecycleDto::Superseded,
    }
}

fn source_from(dto: MemorySourceDto) -> MemorySource {
    match dto {
        MemorySourceDto::ExplicitUserInput => MemorySource::ExplicitUserInput,
        MemorySourceDto::Conversation => MemorySource::Conversation,
        MemorySourceDto::Task => MemorySource::Task,
        MemorySourceDto::Note => MemorySource::Note,
        MemorySourceDto::Idea => MemorySource::Idea,
        MemorySourceDto::Project => MemorySource::Project,
        MemorySourceDto::Document => MemorySource::Document,
        MemorySourceDto::ExternalSource => MemorySource::ExternalSource,
    }
}

fn source_to(source: MemorySource) -> MemorySourceDto {
    match source {
        MemorySource::ExplicitUserInput => MemorySourceDto::ExplicitUserInput,
        MemorySource::Conversation => MemorySourceDto::Conversation,
        MemorySource::Task => MemorySourceDto::Task,
        MemorySource::Note => MemorySourceDto::Note,
        MemorySource::Idea => MemorySourceDto::Idea,
        MemorySource::Project => MemorySourceDto::Project,
        MemorySource::Document => MemorySourceDto::Document,
        MemorySource::ExternalSource => MemorySourceDto::ExternalSource,
    }
}

pub fn to_item(memory: Memory) -> MemoryItem {
    MemoryItem {
        id: memory.id,
        user_id: memory.user_id,
        kind: kind_to(memory.kind),
        lifecycle: lifecycle_to(memory.lifecycle),
        content: memory.content,
        importance: memory.importance.get(),
        confidence: memory.confidence.get(),
        provenance: MemoryProvenanceDto {
            source_kind: source_to(memory.provenance.source_kind),
            source_ref: memory.provenance.source_ref,
        },
        expires_at: memory.expires_at,
        created_at: memory.created_at,
        updated_at: memory.updated_at,
        last_accessed_at: memory.last_accessed_at,
        access_count: memory.access_count,
        archived_at: memory.archived_at,
        superseded_by: memory.superseded_by,
    }
}

fn map_memory_error(error: MemoryError) -> AppError {
    match error {
        MemoryError::NotFound(_) => AppError::NotFound,
        MemoryError::Invalid(reason) => AppError::BadRequest(reason),
        MemoryError::SecretLike => AppError::BadRequest(
            "content looks like a credential and was not stored; please rephrase or remove secrets"
                .into(),
        ),
        MemoryError::Backend(reason) => AppError::Internal(anyhow::anyhow!(reason)),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/memories -- search
// ---------------------------------------------------------------------------

/// Query for `GET /v1/memories`.
///
/// `kind` and `lifecycle` are comma-separated (e.g. `?kind=preference,fact`);
/// axum's default query extractor uses `serde_urlencoded`, which does not
/// support repeated keys for `Vec`, so the API takes lists as a single
/// comma-separated value instead. `min_importance` is 1..=5 and clamped by
/// the store to the valid range; `limit` is bounded server-side.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, deserialize_with = "csv_kinds")]
    pub kind: Vec<MemoryKindDto>,
    #[serde(default, deserialize_with = "csv_lifecycles")]
    pub lifecycle: Vec<MemoryLifecycleDto>,
    #[serde(default)]
    pub min_importance: Option<u8>,
    #[serde(default)]
    pub limit: Option<u32>,
}

fn csv_kinds<'de, D>(deserializer: D) -> Result<Vec<MemoryKindDto>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    csv_of(deserializer)
}

fn csv_lifecycles<'de, D>(deserializer: D) -> Result<Vec<MemoryLifecycleDto>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    csv_of(deserializer)
}

/// Splits a query parameter value on `,` and deserialises each piece via
/// serde. Empty pieces are dropped so a trailing comma is harmless.
fn csv_of<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: serde::de::DeserializeOwned,
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
            serde_json::from_value::<T>(serde_json::Value::String(piece.to_string()))
                .map_err(serde::de::Error::custom)
        })
        .collect()
}

pub async fn list_memories(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<MemoryItem>>, AppError> {
    let store = memory_store(&state)?;

    // Sweep expired temporaries opportunistically. This is the "no scheduler"
    // half of ADR-0035: expiry is enforced on the retrieval path, so a
    // temporary that outlived its `expires_at` is never returned as active.
    let _ = store
        .sweep_expired(principal.user_id, OffsetDateTime::now_utc())
        .await;

    let min_importance = params
        .min_importance
        .map(|value| {
            Importance::new(value.clamp(1, 5))
                .map_err(|error| AppError::BadRequest(error.to_string()))
        })
        .transpose()?;

    let query = MemoryQuery {
        user_id: principal.user_id,
        kinds: params.kind.into_iter().map(kind_from).collect(),
        lifecycles: params.lifecycle.into_iter().map(lifecycle_from).collect(),
        min_importance,
        text: params.q,
        limit: params.limit.unwrap_or(0),
    };

    let rows = store.search(query).await.map_err(map_memory_error)?;
    Ok(Json(rows.into_iter().map(to_item).collect()))
}

// ---------------------------------------------------------------------------
// GET /v1/memories/:id
// ---------------------------------------------------------------------------

pub async fn get_memory(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemoryItem>, AppError> {
    let store = memory_store(&state)?;
    let memory = store
        .get(principal.user_id, id)
        .await
        .map_err(map_memory_error)?;
    Ok(Json(to_item(memory)))
}

// ---------------------------------------------------------------------------
// POST /v1/memories -- create (explicit path)
// ---------------------------------------------------------------------------

pub async fn create_memory(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateMemoryRequest>,
) -> Result<Json<MemoryItem>, AppError> {
    let store = memory_store(&state)?;

    let kind = kind_from(input.kind);
    let importance = Importance::new(input.importance.unwrap_or(3).clamp(1, 5))
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let confidence_value = input.confidence.unwrap_or(1.0).clamp(0.0, 1.0);
    let confidence = Confidence::new(confidence_value)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;

    let source_kind = input
        .source_kind
        .map(source_from)
        .unwrap_or(MemorySource::ExplicitUserInput);
    let provenance = Provenance {
        source_kind,
        source_ref: input.source_ref,
    };

    let new_memory = NewMemory {
        user_id: principal.user_id,
        kind,
        content: input.content,
        importance,
        confidence,
        provenance,
        expires_at: input.expires_at,
    };

    // `create` re-validates; this is the early check that turns obvious
    // problems into a 400 before we round trip to the database.
    new_memory.validate().map_err(map_memory_error)?;

    let stored = store.create(new_memory).await.map_err(map_memory_error)?;

    if let Some(old_id) = input.supersedes {
        store
            .supersede(principal.user_id, old_id, stored.id)
            .await
            .map_err(map_memory_error)?;
    }

    Ok(Json(to_item(stored)))
}

// ---------------------------------------------------------------------------
// PATCH /v1/memories/:id
// ---------------------------------------------------------------------------

pub async fn update_memory(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateMemoryRequest>,
) -> Result<Json<MemoryItem>, AppError> {
    let store = memory_store(&state)?;

    let importance = input
        .importance
        .map(|value| {
            Importance::new(value.clamp(1, 5))
                .map_err(|error| AppError::BadRequest(error.to_string()))
        })
        .transpose()?;
    let confidence = input
        .confidence
        .map(|value| {
            Confidence::new(value.clamp(0.0, 1.0))
                .map_err(|error| AppError::BadRequest(error.to_string()))
        })
        .transpose()?;

    let patch = MemoryPatch {
        kind: input.kind.map(kind_from),
        content: input.content,
        importance,
        confidence,
        expires_at: input.expires_at,
    };

    let memory = store
        .update(principal.user_id, id, patch)
        .await
        .map_err(map_memory_error)?;
    Ok(Json(to_item(memory)))
}

// ---------------------------------------------------------------------------
// POST /v1/memories/:id/archive and .../restore
// ---------------------------------------------------------------------------

pub async fn archive_memory(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemoryItem>, AppError> {
    let store = memory_store(&state)?;
    let memory = store
        .archive(principal.user_id, id)
        .await
        .map_err(map_memory_error)?;
    Ok(Json(to_item(memory)))
}

pub async fn restore_memory(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<Json<MemoryItem>, AppError> {
    let store = memory_store(&state)?;
    let memory = store
        .restore(principal.user_id, id)
        .await
        .map_err(map_memory_error)?;
    Ok(Json(to_item(memory)))
}
