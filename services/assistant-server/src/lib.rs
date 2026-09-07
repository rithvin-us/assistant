//! Assistant server library.
//!
//! The binary is a thin wrapper so that integration tests can build the same
//! router and run it on an ephemeral port.

pub mod academic;
pub mod auth;
pub mod config;
pub mod conversations;
pub mod crypto;
pub mod db;
pub mod documents_store;
pub mod error;
pub mod google;
pub mod memory_store;
pub mod orchestration;
pub mod prompt;
pub mod routes;
pub mod state;
pub mod store;

use std::sync::Arc;

use assistant_auth::{DevTokenVerifier, SupabaseJwtVerifier, TokenVerifier};
use assistant_core::EventBus;
use axum::{Router, extract::Request, http::HeaderValue};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{config::Config, state::AppState};

/// Builds the fully-layered application from already-resolved dependencies.
///
/// The caller owns the [`EventBus`] so that startup and shutdown events can be
/// published from outside the request path, and supplies the orchestrator's
/// dependencies -- see [`orchestration::Dependencies`], whose default is a
/// deployment with no model provider and no tools.
pub fn app(
    config: &Config,
    db: Option<sqlx::PgPool>,
    events: EventBus,
    deps: orchestration::Dependencies,
) -> Router {
    let (orchestrator, approvals) = orchestration::build(
        deps,
        events.clone(),
        orchestration::Settings::from_config(config),
    );

    let http = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    // Real authentication when a Supabase project is configured, and the
    // development placeholder only when one is not. Selecting it takes a
    // deliberate omission rather than being the silent default (ADR-0024).
    let verifier: Arc<dyn TokenVerifier> = match config.supabase_project_ref.as_deref() {
        Some(project_ref) => Arc::new(SupabaseJwtVerifier::for_project(project_ref, http.clone())),
        None => {
            tracing::warn!(
                "SUPABASE_PROJECT_REF is unset; falling back to the development bearer token, which authenticates every caller as one fixed user"
            );
            Arc::new(DevTokenVerifier::new(config.dev_auth_token.clone()))
        }
    };

    let google = db.as_ref().map(|pool| {
        Arc::new(crate::google::GoogleClient::new(
            pool.clone(),
            http.clone(),
            config.google_client_id.clone(),
            config.google_client_secret.clone(),
            config.resolved_encryption_key(),
        ))
    });

    let memory: Option<Arc<dyn assistant_memory::MemoryStore>> = db.as_ref().map(|pool| {
        let store: Arc<dyn assistant_memory::MemoryStore> =
            Arc::new(crate::memory_store::PostgresMemoryStore::new(pool.clone()));
        store
    });

    // M8: document pipeline. Present whenever a database is; the object store
    // defaults to the local filesystem, and the OCR/vision providers default
    // to null (record-and-move-on) implementations, which is honest for a
    // deployment that has neither. See ADR-0036.
    let documents = db.as_ref().map(|pool| {
        let store: Arc<dyn assistant_documents::DocumentStore> = Arc::new(
            crate::documents_store::PostgresDocumentStore::new(pool.clone()),
        );
        let storage: Arc<dyn assistant_documents::DocumentStorage> =
            Arc::new(crate::documents_store::LocalFilesystemStorage::new(
                config.document_storage_dir.clone(),
            ));
        assistant_documents::pipeline::Pipeline {
            store,
            storage,
            ocr: Arc::new(assistant_documents::NullOcrProvider),
            vision: Arc::new(assistant_documents::NullVisionProvider),
        }
    });

    let state = Arc::new(AppState {
        verifier,
        events,
        orchestrator: Arc::new(orchestrator),
        approvals,
        db,
        memory,
        documents,
        http,
        openai_api_key: config.openai_api_key.clone(),
        openai_transcription_model: config.openai_transcription_model.clone(),
        openai_transcription_language: config.openai_transcription_language.clone(),
        google,
        google_redirect_uri: config.google_redirect_uri.clone(),
    });

    let origins: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();

    routes::router(state)
        .layer(
            // The span records `uri.path()`, never the full URI.
            //
            // This is not cosmetic. The conversation WebSocket accepts its
            // bearer token as `?access_token=` -- browsers cannot set headers on
            // a WebSocket handshake -- and `tower_http`'s default span records
            // the whole URI, query string included. That writes a live
            // credential into every request log line. Do not replace this with
            // `TraceLayer::new_for_http()` alone.
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::info_span!(
                    "request",
                    method = %request.method(),
                    path = %request.uri().path(),
                    version = ?request.version(),
                )
            }),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_headers(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any),
        )
}
