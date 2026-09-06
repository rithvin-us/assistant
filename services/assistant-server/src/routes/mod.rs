//! HTTP and WebSocket routing.
//!
//! Routes are grouped by whether they need authentication. Only `/v1/health` is
//! public, so a device can diagnose connectivity before it has a token.

pub mod conversation;
pub mod health;
pub mod productivity;
pub mod transcribe;

use axum::{
    Router, middleware,
    routing::{get, patch, post},
};

use crate::{auth, state::SharedState};

pub fn router(state: SharedState) -> Router {
    let public = Router::new().route("/v1/health", get(health::health));

    let protected = Router::new()
        .route("/v1/conversation/{id}/stream", get(conversation::stream))
        .route("/v1/audio/transcribe", post(transcribe::transcribe))
        // Tasks
        .route(
            "/v1/tasks",
            get(productivity::list_tasks).post(productivity::create_task),
        )
        .route(
            "/v1/tasks/{id}",
            patch(productivity::update_task).delete(productivity::delete_task),
        )
        // Reminders
        .route(
            "/v1/reminders",
            get(productivity::list_reminders).post(productivity::create_reminder),
        )
        .route(
            "/v1/reminders/{id}",
            patch(productivity::update_reminder).delete(productivity::delete_reminder),
        )
        // Notes
        .route(
            "/v1/notes",
            get(productivity::list_notes).post(productivity::create_note),
        )
        .route(
            "/v1/notes/{id}",
            patch(productivity::update_note).delete(productivity::delete_note),
        )
        // Ideas
        .route(
            "/v1/ideas",
            get(productivity::list_ideas).post(productivity::create_idea),
        )
        .route(
            "/v1/ideas/{id}",
            patch(productivity::update_idea).delete(productivity::delete_idea),
        )
        .route(
            "/v1/ideas/{id}/convert",
            post(productivity::convert_idea_to_task),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ));

    public.merge(protected).with_state(state)
}
