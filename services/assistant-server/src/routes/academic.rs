//! HTTP handlers for Classroom, Drive and the unified academic context.
//!
//! These are transport glue in the sense ADR-0001 means it: they read the
//! authenticated principal, hand the work to `crate::academic` or to a
//! provider, and serialise the answer. The decisions — which account may be
//! read, whether a file is small enough, whether a sync may overwrite a title —
//! all live below this layer, so a tool calling the same functions behaves
//! identically to a person tapping the screen.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use assistant_auth::Principal;
use assistant_protocol::{
    AcademicOverview, AcademicSyncResult, Announcement, Course, CourseworkItem, DriveFile,
    DriveFileContent,
};
use assistant_tools::{ClassroomProvider, DriveProvider, ToolError};

use crate::{academic, error::AppError, google::GoogleClient, state::SharedState};

fn google_client(state: &SharedState) -> Result<Arc<GoogleClient>, AppError> {
    state.google.as_ref().cloned().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "Google integration is not available or unconfigured"
        ))
    })
}

fn db(state: &SharedState) -> Result<&sqlx::PgPool, AppError> {
    state
        .db
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("database unavailable")))
}

/// Maps a provider error to a status.
///
/// `ToolError::NotFound` covers both "no such account" and "not your account",
/// deliberately: a caller must not be able to tell the difference and use it to
/// enumerate other users' account ids.
fn map_tool_error(err: ToolError) -> AppError {
    match err {
        ToolError::NotFound(_) => AppError::NotFound,
        ToolError::InvalidArguments(msg) => AppError::BadRequest(msg),
        ToolError::Timeout { .. } => AppError::Internal(anyhow::anyhow!("provider timed out")),
        ToolError::Failed(msg) => AppError::BadRequest(msg),
    }
}

// ---------------------------------------------------------------------------
// Classroom
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AccountQuery {
    pub account_id: Uuid,
    /// When true, ask Google. Otherwise answer from the cache, which is what
    /// makes the screen usable offline.
    #[serde(default)]
    pub refresh: bool,
}

pub async fn list_courses(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<AccountQuery>,
) -> Result<Json<Vec<Course>>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    if query.refresh {
        let client = google_client(&state)?;
        let courses = ClassroomProvider::courses(&*client, query.account_id, principal.user_id)
            .await
            .map_err(map_tool_error)?;
        return Ok(Json(courses));
    }

    let courses = academic::cached_courses(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;
    Ok(Json(courses))
}

#[derive(Debug, Deserialize)]
pub struct CourseworkQuery {
    pub account_id: Uuid,
    /// Absent means "every course", which is what the overview needs.
    pub course_id: Option<String>,
    #[serde(default)]
    pub refresh: bool,
}

pub async fn list_coursework(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<CourseworkQuery>,
) -> Result<Json<Vec<CourseworkItem>>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    if query.refresh {
        let course_id = query.course_id.clone().ok_or_else(|| {
            AppError::BadRequest("course_id is required when refreshing coursework".into())
        })?;
        let client = google_client(&state)?;
        let items = ClassroomProvider::coursework(
            &*client,
            query.account_id,
            principal.user_id,
            &course_id,
        )
        .await
        .map_err(map_tool_error)?;
        return Ok(Json(items));
    }

    let items = academic::cached_coursework(
        pool,
        principal.user_id,
        query.account_id,
        query.course_id.as_deref(),
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(items))
}

#[derive(Debug, Deserialize)]
pub struct AnnouncementQuery {
    pub account_id: Uuid,
    pub course_id: Option<String>,
    pub limit: Option<u32>,
    #[serde(default)]
    pub refresh: bool,
}

pub async fn list_announcements(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<AnnouncementQuery>,
) -> Result<Json<Vec<Announcement>>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    let limit = query.limit.unwrap_or(20);

    if query.refresh {
        let course_id = query.course_id.clone().ok_or_else(|| {
            AppError::BadRequest("course_id is required when refreshing announcements".into())
        })?;
        let client = google_client(&state)?;
        let items = ClassroomProvider::announcements(
            &*client,
            query.account_id,
            principal.user_id,
            &course_id,
            limit,
        )
        .await
        .map_err(map_tool_error)?;
        return Ok(Json(items));
    }

    let items = academic::cached_announcements(
        pool,
        principal.user_id,
        query.account_id,
        query.course_id.as_deref(),
        limit as i64,
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(items))
}

// ---------------------------------------------------------------------------
// Drive
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct DriveSearchQuery {
    pub account_id: Uuid,
    #[serde(default)]
    pub q: String,
    pub mime_type: Option<String>,
    pub limit: Option<u32>,
}

pub async fn search_drive(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<DriveSearchQuery>,
) -> Result<Json<Vec<DriveFile>>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    let client = google_client(&state)?;
    let files = DriveProvider::search(
        &*client,
        query.account_id,
        principal.user_id,
        &query.q,
        query.mime_type.as_deref(),
        query.limit.unwrap_or(25),
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(files))
}

#[derive(Debug, Deserialize)]
pub struct DriveListQuery {
    pub account_id: Uuid,
    pub folder_id: Option<String>,
    pub limit: Option<u32>,
}

pub async fn list_drive(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<DriveListQuery>,
) -> Result<Json<Vec<DriveFile>>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    let client = google_client(&state)?;
    let files = DriveProvider::list(
        &*client,
        query.account_id,
        principal.user_id,
        query.folder_id.as_deref(),
        query.limit.unwrap_or(50),
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(files))
}

#[derive(Debug, Deserialize)]
pub struct DriveFileQuery {
    pub account_id: Uuid,
}

pub async fn drive_metadata(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(file_id): Path<String>,
    Query(query): Query<DriveFileQuery>,
) -> Result<Json<DriveFile>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    let client = google_client(&state)?;
    let file = DriveProvider::metadata(&*client, query.account_id, principal.user_id, &file_id)
        .await
        .map_err(map_tool_error)?;
    Ok(Json(file))
}

/// Reads a small text file.
///
/// The size and MIME checks happen in the provider, before any download
/// starts, and a refusal comes back as a 400 with a sentence the user can
/// read — never an empty body that would look like a successfully read empty
/// file.
pub async fn drive_read(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(file_id): Path<String>,
    Query(query): Query<DriveFileQuery>,
) -> Result<Json<DriveFileContent>, AppError> {
    let pool = db(&state)?;
    academic::assert_account_active(pool, principal.user_id, query.account_id)
        .await
        .map_err(map_tool_error)?;

    let client = google_client(&state)?;
    let content =
        DriveProvider::read_small_file(&*client, query.account_id, principal.user_id, &file_id)
            .await
            .map_err(map_tool_error)?;
    Ok(Json(content))
}

// ---------------------------------------------------------------------------
// Academic context
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OverviewQuery {
    pub limit: Option<i64>,
}

/// The Academic Overview. Cache only — never calls Google, so it is instant
/// and works offline.
pub async fn academic_overview(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<OverviewQuery>,
) -> Result<Json<AcademicOverview>, AppError> {
    let pool = db(&state)?;
    let overview = academic::overview(pool, principal.user_id, query.limit.unwrap_or(10))
        .await
        .map_err(map_tool_error)?;
    Ok(Json(overview))
}

#[derive(Debug, Deserialize)]
pub struct SyncInput {
    pub account_id: Uuid,
    /// Announcements fetched per course. Zero skips announcements entirely,
    /// which keeps a deadline-only sync cheaper against the Classroom quota.
    pub announcements_per_course: Option<u32>,
}

/// Explicit refresh: pulls Classroom for one account and imports coursework
/// into tasks.
///
/// Triggered by the user, never on a timer. Polling someone else's API on a
/// schedule burns quota for data that mostly has not changed; see ADR-0034.
pub async fn academic_sync(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<SyncInput>,
) -> Result<Json<AcademicSyncResult>, AppError> {
    let pool = db(&state)?;
    let client = google_client(&state)?;

    let result = academic::sync_account(
        pool,
        &*client,
        principal.user_id,
        input.account_id,
        input.announcements_per_course.unwrap_or(5),
    )
    .await
    .map_err(map_tool_error)?;

    Ok(Json(result))
}
