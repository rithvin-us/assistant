//! Shared application state.
//!
//! Held in an `Arc` and cloned into every handler. It carries seams (the auth
//! verifier, the event bus), not business logic.

use std::sync::Arc;

use assistant_auth::TokenVerifier;
use assistant_core::{EventBus, Orchestrator, actions::ApprovalCoordinator};
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
    pub http: reqwest::Client,
    pub openai_api_key: Option<String>,
    pub openai_transcription_model: String,
    pub openai_transcription_language: Option<String>,
    pub google: Option<Arc<GoogleClient>>,
    pub google_redirect_uri: Option<String>,
}

impl AppState {
    pub fn is_healthy(&self) -> bool {
        self.db.is_some()
    }
}
