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
    /// A dependency this request needs is not available -- most often the
    /// database. Distinct from `Internal` because it is not a bug in this
    /// server and a retry may succeed, which a client cannot infer from a 500.
    #[error("{0} is unavailable")]
    DependencyUnavailable(&'static str),
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
            Self::DependencyUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "dependency_unavailable")
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// M13 regression. A database outage used to surface as 500 `internal`,
    /// which tells a client "this server is broken" when the truth is "a
    /// dependency is down and a retry may succeed". The two must not share a
    /// code.
    #[test]
    fn a_missing_dependency_is_503_and_not_an_internal_error() {
        let (status, code) = AppError::DependencyUnavailable("the database").parts();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code, "dependency_unavailable");

        let (internal_status, internal_code) = AppError::Internal(anyhow::anyhow!("boom")).parts();
        assert_ne!(status, internal_status);
        assert_ne!(code, internal_code);
    }

    /// Every arm maps to a distinct machine-readable code, so a client never
    /// has to parse the human-readable message to tell them apart.
    #[test]
    fn every_error_has_its_own_code() {
        let codes = [
            AppError::Unauthorized.parts().1,
            AppError::NotFound.parts().1,
            AppError::BadRequest(String::new()).parts().1,
            AppError::DependencyUnavailable("x").parts().1,
            AppError::Internal(anyhow::anyhow!("x")).parts().1,
        ];
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len(), "codes collide: {codes:?}");
    }

    /// The dependency name is a `&'static str` chosen in this crate, never a
    /// formatted cause, so a connection string cannot reach a response body.
    #[test]
    fn a_dependency_message_carries_no_cause() {
        let message = AppError::DependencyUnavailable("the database").to_string();
        assert_eq!(message, "the database is unavailable");
    }
}
