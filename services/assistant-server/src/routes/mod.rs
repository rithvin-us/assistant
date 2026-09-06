//! HTTP and WebSocket routing.
//!
//! Routes are grouped by whether they need authentication. Only `/v1/health` is
//! public, so a device can diagnose connectivity before it has a token.

pub mod conversation;
pub mod health;
pub mod transcribe;

use axum::{
    Router, middleware,
    routing::{get, post},
};

use crate::{auth, state::SharedState};

pub fn router(state: SharedState) -> Router {
    let public = Router::new().route("/v1/health", get(health::health));

    let protected = Router::new()
        .route("/v1/conversation/{id}/stream", get(conversation::stream))
        .route("/v1/audio/transcribe", post(transcribe::transcribe))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ));

    public.merge(protected).with_state(state)
}
