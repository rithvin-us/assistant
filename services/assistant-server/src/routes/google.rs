//! HTTP handlers for Google OAuth, multi-account management, Gmail, Calendar, and Free-Time slots.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

use assistant_auth::Principal;
use assistant_protocol::{
    AccountSummary, CalendarEvent, CreateEventRequest as CreateCalendarEvent, EmailDetail,
    EmailSummary, FreeSlot,
};
use assistant_tools::{CalendarProvider, GmailProvider, ToolError, providers::UpdateCalendarEvent};

use crate::{
    error::AppError,
    google::{GoogleClient, find_free_slots},
    state::SharedState,
};

fn google_client(state: &SharedState) -> Result<Arc<GoogleClient>, AppError> {
    state.google.as_ref().cloned().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "Google integration is not available or unconfigured"
        ))
    })
}

fn map_tool_error(err: ToolError) -> AppError {
    match err {
        ToolError::NotFound(_) => AppError::NotFound,
        ToolError::InvalidArguments(msg) => AppError::BadRequest(msg),
        ToolError::Timeout { .. } => AppError::Internal(anyhow::anyhow!("{err}")),
        ToolError::Failed(msg) => {
            if msg.contains("not found") {
                AppError::NotFound
            } else if msg.contains("Permission denied")
                || msg.contains("unauthorized")
                || msg.contains("auth")
            {
                AppError::Unauthorized
            } else {
                AppError::BadRequest(msg)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OAuth & Accounts
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StartOAuthInput {
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartOAuthResponse {
    pub auth_url: String,
}

pub async fn start_oauth(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<StartOAuthInput>,
) -> Result<Json<StartOAuthResponse>, AppError> {
    let client = google_client(&state)?;
    let redirect = input
        .redirect_uri
        .as_deref()
        .or(state.google_redirect_uri.as_deref())
        .ok_or_else(|| AppError::BadRequest("Missing redirect_uri".into()))?;

    let auth_url = client
        .generate_auth_url(principal.user_id, redirect)
        .map_err(map_tool_error)?;

    Ok(Json(StartOAuthResponse { auth_url }))
}

#[derive(Debug, Deserialize)]
pub struct ExchangeOAuthInput {
    pub code: String,
    pub redirect_uri: String,
}

pub async fn exchange_oauth(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<ExchangeOAuthInput>,
) -> Result<Json<AccountSummary>, AppError> {
    let client = google_client(&state)?;
    let account = client
        .exchange_code(principal.user_id, &input.code, &input.redirect_uri)
        .await
        .map_err(map_tool_error)?;

    Ok(Json(account))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Public callback handler when Google redirects back after authorization.
pub async fn oauth_callback(
    State(state): State<SharedState>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if let Some(err) = query.error {
        return (
            StatusCode::BAD_REQUEST,
            Html(format!(
                "<html><body style='font-family:sans-serif;text-align:center;padding:40px;'>\
                <h2>Google Authorization Cancelled or Failed</h2>\
                <p>Error: {}</p>\
                </body></html>",
                html_escape(&err)
            )),
        )
            .into_response();
    }

    if query.code.is_none() && query.state.is_none() {
        return (
            StatusCode::OK,
            Html(
                "<!DOCTYPE html><html><body style='font-family:sans-serif;text-align:center;padding:50px;'>\
                <h2 style='color:#1a73e8;'>Assistant Server</h2>\
                <p style='color:#555;'>Server is running.</p>\
                </body></html>",
            ),
        )
            .into_response();
    }

    let (Some(code), Some(state_param)) = (query.code, query.state) else {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h2>Missing code or state parameter</h2>"),
        )
            .into_response();
    };

    let client = match google_client(&state) {
        Ok(c) => c,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("<h2>Google integration not configured on server</h2>"),
            )
                .into_response();
        }
    };

    let user_id = match client.verify_oauth_state(&state_param) {
        Ok(uid) => uid,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Html(format!("<h2>Invalid or expired state</h2><p>{e}</p>")),
            )
                .into_response();
        }
    };

    let redirect_uri = state
        .google_redirect_uri
        .clone()
        .unwrap_or_else(|| "http://localhost:8787/v1/auth/google/callback".into());

    match client.exchange_code(user_id, &code, &redirect_uri).await {
        Ok(acc) => (
            StatusCode::OK,
            Html(format!(
                "<!DOCTYPE html><html><body style='font-family:sans-serif;text-align:center;padding:50px;background:#f9f9f9;'>\
                <div style='max-width:400px;margin:auto;background:white;padding:30px;border-radius:12px;box-shadow:0 2px 8px rgba(0,0,0,0.08);'>\
                <h2 style='color:#1a73e8;margin-top:0;'>Account Connected!</h2>\
                <p>Connected <strong>{}</strong> successfully.</p>\
                <p style='color:#666;'>You can close this tab and return to the assistant app.</p>\
                </div>\
                <script>if (window.opener) {{ window.close(); }}</script>\
                </body></html>",
                html_escape(&acc.email)
            )),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Html(format!("<h2>Connection failed</h2><p>{e}</p>")),
        )
            .into_response(),
    }
}

pub async fn list_accounts(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Vec<AccountSummary>>, AppError> {
    let client = match state.google.as_ref() {
        Some(c) => c,
        None => return Ok(Json(Vec::new())),
    };
    let accounts = client
        .list_accounts(principal.user_id)
        .await
        .map_err(map_tool_error)?;
    Ok(Json(accounts))
}

pub async fn disconnect_account(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let client = google_client(&state)?;
    client
        .disconnect_account(principal.user_id, id)
        .await
        .map_err(map_tool_error)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Gmail
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GmailSearchQuery {
    pub account_id: Uuid,
    pub q: String,
    pub limit: Option<u32>,
}

pub async fn search_gmail(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<GmailSearchQuery>,
) -> Result<Json<Vec<EmailSummary>>, AppError> {
    let client = google_client(&state)?;
    let limit = query.limit.unwrap_or(20);
    let results = GmailProvider::search(
        &*client,
        query.account_id,
        principal.user_id,
        &query.q,
        limit,
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(results))
}

#[derive(Debug, Deserialize)]
pub struct GmailReadQuery {
    pub account_id: Uuid,
}

pub async fn read_gmail(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(message_id): Path<String>,
    Query(query): Query<GmailReadQuery>,
) -> Result<Json<EmailDetail>, AppError> {
    let client = google_client(&state)?;
    let detail = GmailProvider::read(&*client, query.account_id, principal.user_id, &message_id)
        .await
        .map_err(map_tool_error)?;
    Ok(Json(detail))
}

// ---------------------------------------------------------------------------
// Calendar & Free Time
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CalendarListQuery {
    pub account_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub time_min: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub time_max: OffsetDateTime,
}

pub async fn list_calendar_events(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<CalendarListQuery>,
) -> Result<Json<Vec<CalendarEvent>>, AppError> {
    let client = google_client(&state)?;
    let events = CalendarProvider::list(
        &*client,
        query.account_id,
        principal.user_id,
        query.time_min,
        query.time_max,
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(events))
}

#[derive(Debug, Deserialize)]
pub struct CalendarAccountQuery {
    pub account_id: Uuid,
}

pub async fn create_calendar_event(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<CalendarAccountQuery>,
    Json(body): Json<CreateCalendarEvent>,
) -> Result<Json<CalendarEvent>, AppError> {
    let client = google_client(&state)?;
    let event = CalendarProvider::create(&*client, query.account_id, principal.user_id, body)
        .await
        .map_err(map_tool_error)?;
    Ok(Json(event))
}

pub async fn update_calendar_event(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(event_id): Path<String>,
    Query(query): Query<CalendarAccountQuery>,
    Json(body): Json<UpdateCalendarEvent>,
) -> Result<Json<CalendarEvent>, AppError> {
    let client = google_client(&state)?;
    let event = CalendarProvider::update(
        &*client,
        query.account_id,
        principal.user_id,
        &event_id,
        body,
    )
    .await
    .map_err(map_tool_error)?;
    Ok(Json(event))
}

pub async fn delete_calendar_event(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Path(event_id): Path<String>,
    Query(query): Query<CalendarAccountQuery>,
) -> Result<StatusCode, AppError> {
    let client = google_client(&state)?;
    CalendarProvider::delete(&*client, query.account_id, principal.user_id, &event_id)
        .await
        .map_err(map_tool_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct FreeSlotsQuery {
    pub account_id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub duration_minutes: u32,
}

pub async fn get_free_slots(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<FreeSlotsQuery>,
) -> Result<Json<Vec<FreeSlot>>, AppError> {
    let client = google_client(&state)?;
    let events = CalendarProvider::list(
        &*client,
        query.account_id,
        principal.user_id,
        query.start_time,
        query.end_time,
    )
    .await
    .map_err(map_tool_error)?;

    let slots = find_free_slots(
        &events,
        query.start_time,
        query.end_time,
        query.duration_minutes,
        None,
    );

    Ok(Json(slots))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
