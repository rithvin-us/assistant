//! Assistant server library.
//!
//! The binary is a thin wrapper so that integration tests can build the same
//! router and run it on an ephemeral port.

pub mod auth;
pub mod config;
pub mod conversations;
pub mod db;
pub mod error;
pub mod orchestration;
pub mod prompt;
pub mod routes;
pub mod state;
pub mod store;

use std::sync::Arc;

use assistant_auth::DevTokenVerifier;
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

    let state = Arc::new(AppState {
        verifier: Arc::new(DevTokenVerifier::new(config.dev_auth_token.clone())),
        events,
        orchestrator: Arc::new(orchestrator),
        approvals,
        db,
        http,
        openai_api_key: config.openai_api_key.clone(),
        openai_transcription_model: config.openai_transcription_model.clone(),
        openai_transcription_language: config.openai_transcription_language.clone(),
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
        .layer(CorsLayer::new().allow_origin(origins))
}
