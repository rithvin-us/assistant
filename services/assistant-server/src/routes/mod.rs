//! HTTP and WebSocket routing.
//!
//! Routes are grouped by whether they need authentication. Only `/v1/health` and
//! the OAuth callback are public.

pub mod academic;
pub mod conversation;
pub mod google;
pub mod health;
pub mod memory;
pub mod productivity;
pub mod transcribe;

use axum::{
    Router, middleware,
    routing::{delete, get, patch, post},
};

use crate::{auth, state::SharedState};

pub fn router(state: SharedState) -> Router {
    let public = Router::new()
        .route("/v1/health", get(health::health))
        .route("/v1/auth/google/callback", get(google::oauth_callback));

    let protected = Router::new()
        .route("/v1/conversation/{id}/stream", get(conversation::stream))
        .route("/v1/audio/transcribe", post(transcribe::transcribe))
        // Projects
        .route(
            "/v1/projects",
            get(productivity::list_projects).post(productivity::create_project),
        )
        .route(
            "/v1/projects/{id}",
            patch(productivity::update_project).delete(productivity::delete_project),
        )
        // Labels
        .route(
            "/v1/labels",
            get(productivity::list_labels).post(productivity::create_label),
        )
        .route(
            "/v1/labels/{id}",
            patch(productivity::update_label).delete(productivity::delete_label),
        )
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
        // Google OAuth & Multi-Accounts
        .route("/v1/auth/google/start", post(google::start_oauth))
        .route("/v1/auth/google/exchange", post(google::exchange_oauth))
        .route("/v1/google/accounts", get(google::list_accounts))
        .route(
            "/v1/google/accounts/{id}",
            delete(google::disconnect_account),
        )
        // Gmail
        .route("/v1/google/gmail/search", get(google::search_gmail))
        .route("/v1/google/gmail/messages/{id}", get(google::read_gmail))
        // Calendar & Schedule
        .route(
            "/v1/google/calendar/events",
            get(google::list_calendar_events).post(google::create_calendar_event),
        )
        .route(
            "/v1/google/calendar/events/{id}",
            patch(google::update_calendar_event).delete(google::delete_calendar_event),
        )
        .route(
            "/v1/google/calendar/free-slots",
            get(google::get_free_slots),
        )
        // Classroom (read-only)
        .route("/v1/classroom/courses", get(academic::list_courses))
        .route("/v1/classroom/coursework", get(academic::list_coursework))
        .route(
            "/v1/classroom/announcements",
            get(academic::list_announcements),
        )
        // Drive (read-only)
        .route("/v1/drive/search", get(academic::search_drive))
        .route("/v1/drive/files", get(academic::list_drive))
        .route("/v1/drive/files/{id}", get(academic::drive_metadata))
        .route("/v1/drive/files/{id}/content", get(academic::drive_read))
        // Unified academic context
        .route("/v1/academic/overview", get(academic::academic_overview))
        .route("/v1/academic/sync", post(academic::academic_sync))
        // Long-term memory
        .route(
            "/v1/memories",
            get(memory::list_memories).post(memory::create_memory),
        )
        .route(
            "/v1/memories/{id}",
            get(memory::get_memory).patch(memory::update_memory),
        )
        .route("/v1/memories/{id}/archive", post(memory::archive_memory))
        .route("/v1/memories/{id}/restore", post(memory::restore_memory))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ));

    public.merge(protected).with_state(state)
}
