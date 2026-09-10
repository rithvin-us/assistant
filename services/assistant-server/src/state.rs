//! Shared application state.
//!
//! Held in an `Arc` and cloned into every handler. It carries seams (the auth
//! verifier, the event bus), not business logic.

use std::sync::Arc;

use assistant_auth::TokenVerifier;
use assistant_core::{EventBus, Orchestrator, actions::ApprovalCoordinator};
use assistant_documents::pipeline::Pipeline as DocumentPipeline;
use assistant_memory::MemoryStore;
use sqlx::PgPool;

use crate::google::GoogleClient;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub verifier: Arc<dyn TokenVerifier>,
    pub events: EventBus,
    /// The execution spine. Handlers construct a `TurnRequest` and hand it over;
    /// no orchestration logic lives in the transport layer.
    pub orchestrator: Arc<Orchestrator>,
    /// Resumes approved actions. `None` without a durable store.
    pub approvals: Option<Arc<ApprovalCoordinator>>,
    /// `None` when no `DATABASE_URL` was configured.
    pub db: Option<PgPool>,
    /// Long-term memory store. `None` when there is no database.
    pub memory: Option<Arc<dyn MemoryStore>>,
    /// M8 document pipeline (store + storage + OCR/vision providers). `None`
    /// when there is no database.
    pub documents: Option<DocumentPipeline>,
    pub http: reqwest::Client,
    pub openai_api_key: Option<String>,
    pub gemini_api_key: Option<String>,
    pub openai_base_url: Option<String>,
    pub model: String,
    pub openai_transcription_model: String,
    pub openai_transcription_language: Option<String>,
    pub google: Option<Arc<GoogleClient>>,
    pub google_redirect_uri: Option<String>,
    pub cartesia_api_key: Option<String>,
    pub cartesia_stt_model: String,
    pub cartesia_tts_model: String,
    pub cartesia_tts_voice_id: String,
    /// Bounds how many paid voice calls one principal can make.
    pub voice_rate_limiter: Arc<crate::rate_limit::RateLimiter>,
}

/// How long the readiness probe waits for the database before calling it
/// unready. Short, because readiness is polled.
const READINESS_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

impl AppState {
    /// Whether the database is configured *and* answering right now.
    ///
    /// This replaces an earlier `is_healthy` that returned `self.db.is_some()`.
    /// That only proved a pool had been constructed at boot, so a database that
    /// fell over afterwards still reported healthy -- the process was alive and
    /// the endpoint said so, which is the failure this asks a real question to
    /// avoid.
    pub async fn is_ready(&self) -> bool {
        let Some(pool) = self.db.as_ref() else {
            return false;
        };
        matches!(
            tokio::time::timeout(
                READINESS_PROBE_TIMEOUT,
                sqlx::query("select 1").execute(pool)
            )
            .await,
            Ok(Ok(_))
        )
    }
}
