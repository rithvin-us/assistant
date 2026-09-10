//! HTTP and WebSocket routing.
//!
//! Routes are grouped by whether they need authentication. Only `/v1/health` and
//! the OAuth callback are public.

pub mod academic;
pub mod conversation;
pub mod documents;
pub mod google;
pub mod health;
pub mod memory;
pub mod planning;
pub mod productivity;
pub mod transcribe;
pub mod voice;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post},
};

use crate::{auth, state::SharedState};

pub fn router(state: SharedState) -> Router {
    let public = Router::new()
        .route("/", get(google::oauth_callback))
        .route("/v1/health", get(health::health))
        .route("/v1/ready", get(health::ready))
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
        // Documents (M8)
        .route(
            "/v1/documents",
            get(documents::list_documents)
                .post(documents::upload_document)
                .layer(DefaultBodyLimit::max(
                    (assistant_documents::MAX_DOCUMENT_BYTES + 1024 * 1024) as usize,
                )),
        )
        .route(
            "/v1/documents/{id}",
            get(documents::get_document).delete(documents::delete_document),
        )
        .route("/v1/documents/{id}/pages", get(documents::list_pages))
        .route("/v1/documents/{id}/pages/{n}", get(documents::get_page))
        .route(
            "/v1/documents/{id}/reprocess",
            post(documents::reprocess_document),
        )
        .route("/v1/documents/search", get(documents::search_pages))
        .route(
            "/v1/documents/from-drive",
            post(documents::ingest_from_drive),
        )
        // Unified Personal Planning (M9)
        .route("/v1/planning/today", get(planning::get_today_plan))
        .route(
            "/v1/planning/upcoming",
            get(planning::get_upcoming_planning),
        )
        .route("/v1/planning/analyze", post(planning::analyze_planning))
        .route("/v1/planning/plan", post(planning::generate_plan))
        .route("/v1/planning/conflicts", get(planning::get_conflicts))
        // Voice Integration (M10)
        //
        // Axum's default body limit is 2 MB and nothing overrode it, so the
        // handler's own size check could never be reached and an ordinary
        // recording could be rejected by the framework with no explanatory
        // code. The limit here sits just above the handler's, so the handler
        // answers with a proper `audio_too_large` for anything oversized and
        // this remains only a backstop against a body that should never be
        // buffered at all.
        .route(
            "/v1/voice/transcribe",
            post(voice::transcribe)
                .layer(DefaultBodyLimit::max(voice::MAX_AUDIO_BYTES + 1024 * 1024)),
        )
        .route("/v1/voice/speak", post(voice::speak))
        .route("/v1/voice/diagnostic", get(voice::diagnostic))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_bearer,
        ));

    public.merge(protected).with_state(state)
}
