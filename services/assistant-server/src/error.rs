//! One error type for the whole HTTP surface.
//!
//! Handlers return `Result<T, AppError>`. The `IntoResponse` impl is the only
//! place that decides what a client sees, which keeps internal detail from
//! leaking into responses by accident.

use assistant_protocol::ApiError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication required")]
    Unauthorized,
    #[error("not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    /// Anything unexpected. The cause is logged, never returned.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    fn parts(&self) -> (StatusCode, &'static str) {
        match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.parts();

        let message = match &self {
            // The underlying cause may contain connection strings or query
            // fragments, so it is logged and replaced with a fixed string.
            AppError::Internal(cause) => {
                tracing::error!(error = ?cause, "unhandled internal error");
                "internal server error".to_string()
            }
            other => other.to_string(),
        };

        (
            status,
            Json(ApiError {
                code: code.to_string(),
                message,
            }),
        )
            .into_response()
    }
}
