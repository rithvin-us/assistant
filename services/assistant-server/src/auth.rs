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

use assistant_auth::Principal;
use sqlx::{PgPool, Row};

use crate::{error::AppError, google::client::expand_google_scopes, state::SharedState};

/// Populates `principal.scopes` with expanded scopes from all active connected accounts.
pub async fn populate_principal_scopes(
    pool: &PgPool,
    principal: &mut Principal,
) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT scopes
        FROM connected_accounts
        WHERE user_id = $1 AND status = 'active'
        "#,
    )
    .bind(principal.user_id)
    .fetch_all(pool)
    .await?;

    let mut all_scopes: std::collections::HashSet<String> =
        principal.scopes.iter().cloned().collect();
    for row in rows {
        let scopes: Vec<String> = row.get("scopes");
        for s in expand_google_scopes(&scopes) {
            all_scopes.insert(s);
        }
    }

    let mut sorted: Vec<String> = all_scopes.into_iter().collect();
    sorted.sort();
    principal.scopes = sorted;
    Ok(())
}

pub async fn require_bearer(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = bearer_token(&request).ok_or(AppError::Unauthorized)?;

    let mut principal = state
        .verifier
        .verify(&token)
        .await
        .map_err(|_| AppError::Unauthorized)?;

    if let Some(pool) = &state.db
        && let Err(e) = populate_principal_scopes(pool, &mut principal).await
    {
        tracing::warn!(error = %e, "Failed to populate principal scopes from connected accounts");
    }

    request.extensions_mut().insert(principal);
    Ok(next.run(request).await)
}

/// Reads `Authorization: Bearer <token>`, or the `access_token` query parameter.
///
/// The query fallback exists because a browser `WebSocket` cannot set headers.
/// It is accepted *only* on a WebSocket upgrade, which this function enforces --
/// the doc comment used to claim that while the code accepted the parameter on
/// every route. A credential in a URL reaches proxy access logs, `Referer`
/// headers and browser history, none of which this process controls, so the
/// narrower the path that accepts one the better. This server's own logging is
/// already configured without query strings in `main.rs`.
fn bearer_token(request: &Request) -> Option<String> {
    if let Some(value) = request.headers().get(AUTHORIZATION)
        && let Ok(text) = value.to_str()
        && let Some(token) = text.strip_prefix("Bearer ")
    {
        return Some(token.trim().to_string());
    }

    if !is_websocket_upgrade(request) {
        return None;
    }

    request.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.strip_prefix("access_token="))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
    })
}

/// Whether this request is a WebSocket handshake.
///
/// `Upgrade: websocket` is required by RFC 6455 and is what `axum`'s own
/// `WebSocketUpgrade` extractor checks, so a request that would not upgrade
/// cannot use the query credential.
fn is_websocket_upgrade(request: &Request) -> bool {
    request
        .headers()
        .get(axum::http::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest};

    fn request(uri: &str, upgrade: bool) -> Request {
        let mut builder = HttpRequest::builder().uri(uri);
        if upgrade {
            builder = builder.header(axum::http::header::UPGRADE, "websocket");
        }
        builder.body(Body::empty()).expect("valid request")
    }

    /// M13 regression. The query credential is for browser WebSockets, which
    /// cannot set headers. Accepting it on ordinary routes put a bearer token
    /// into every intermediary's access log for no benefit.
    #[test]
    fn a_query_token_is_ignored_on_an_ordinary_request() {
        assert_eq!(
            bearer_token(&request("/v1/memories?access_token=t", false)),
            None
        );
    }

    #[test]
    fn a_query_token_is_accepted_on_a_websocket_upgrade() {
        assert_eq!(
            bearer_token(&request("/v1/conversation/x/stream?access_token=t", true)).as_deref(),
            Some("t")
        );
    }

    /// The upgrade header is compared case-insensitively, as RFC 6455 allows.
    #[test]
    fn the_upgrade_header_is_matched_case_insensitively() {
        let mut req = request("/s?access_token=t", false);
        req.headers_mut().insert(
            axum::http::header::UPGRADE,
            "WebSocket".parse().expect("value"),
        );
        assert!(bearer_token(&req).is_some());
    }

    /// The header always wins, on any route, so restricting the query fallback
    /// cannot break a normal client.
    #[test]
    fn the_authorization_header_still_works_everywhere() {
        let mut req = request("/v1/tasks", false);
        req.headers_mut()
            .insert(AUTHORIZATION, "Bearer abc".parse().expect("value"));
        assert_eq!(bearer_token(&req).as_deref(), Some("abc"));
    }

    #[test]
    fn an_empty_query_token_is_not_a_credential() {
        assert_eq!(bearer_token(&request("/s?access_token=", true)), None);
    }
}
