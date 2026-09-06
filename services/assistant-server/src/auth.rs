//! Bearer-token extraction middleware.
//!
//! Every route under `/v1` except `/v1/health` passes through here. The
//! resulting `Principal` is put into request extensions so handlers can take
//! it without knowing how it was obtained.

use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};

use crate::{error::AppError, state::SharedState};

pub async fn require_bearer(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = bearer_token(&request).ok_or(AppError::Unauthorized)?;

    let principal = state
        .verifier
        .verify(&token)
        .await
        .map_err(|_| AppError::Unauthorized)?;

    request.extensions_mut().insert(principal);
    Ok(next.run(request).await)
}

/// Reads `Authorization: Bearer <token>`, or the `access_token` query parameter.
///
/// The query fallback exists because browser `WebSocket` cannot set headers. It
/// is accepted only on the WebSocket upgrade path and the value must never be
/// logged, so `tower_http`'s request logging is configured without query
/// strings in `main.rs`.
fn bearer_token(request: &Request) -> Option<String> {
    if let Some(value) = request.headers().get(AUTHORIZATION)
        && let Ok(text) = value.to_str()
        && let Some(token) = text.strip_prefix("Bearer ")
    {
        return Some(token.trim().to_string());
    }

    request.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.strip_prefix("access_token="))
            .map(|t| t.to_string())
    })
}
