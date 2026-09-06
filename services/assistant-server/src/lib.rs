//! Assistant server library.
//!
//! The binary is a thin wrapper so that integration tests can build the same
//! router and run it on an ephemeral port.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod routes;
pub mod state;

use std::sync::Arc;

use assistant_auth::DevTokenVerifier;
use assistant_core::EventBus;
use axum::{Router, http::HeaderValue};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{config::Config, state::AppState};

/// Builds the fully-layered application from already-resolved dependencies.
///
/// The caller owns the [`EventBus`] so that startup and shutdown events can be
/// published from outside the request path.
pub fn app(config: &Config, db: Option<sqlx::PgPool>, events: EventBus) -> Router {
    let state = Arc::new(AppState {
        verifier: Arc::new(DevTokenVerifier::new(config.dev_auth_token.clone())),
        events,
        db,
    });

    let origins: Vec<HeaderValue> = config
        .allowed_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();

    routes::router(state)
        .layer(
            // `make_span_with` is left at its default, which records the method and
            // path but not the query string, so a token passed as `access_token`
            // never reaches the logs.
            TraceLayer::new_for_http(),
        )
        .layer(CorsLayer::new().allow_origin(origins))
}
