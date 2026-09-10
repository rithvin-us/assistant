use assistant_protocol::{HealthResponse, HealthStatus, PROTOCOL_VERSION};
use axum::{Json, extract::State, http::StatusCode};
use time::OffsetDateTime;

use crate::state::SharedState;

/// Public liveness and capability probe.
///
/// Always answers 200 so the mobile app can distinguish "server unreachable"
/// from "server up, storage not working"; the distinction is carried in the
/// body rather than in the status code. `/v1/ready` is the endpoint that
/// answers with a status code.
pub async fn health(State(state): State<SharedState>) -> Json<HealthResponse> {
    Json(report(&state).await)
}

/// Readiness, for anything that reads a status code rather than a body: 200
/// when the database is answering, 503 when it is not.
///
/// A live process is not the same thing as a working one. Keeping this separate
/// from `/v1/health` means a platform health check can be pointed at the
/// question it actually means to ask.
pub async fn ready(State(state): State<SharedState>) -> (StatusCode, Json<HealthResponse>) {
    let body = report(&state).await;
    let code = match body.status {
        HealthStatus::Ok => StatusCode::OK,
        HealthStatus::Degraded => StatusCode::SERVICE_UNAVAILABLE,
    };
    (code, Json(body))
}

/// The one place the status is decided, so the two endpoints cannot disagree.
async fn report(state: &SharedState) -> HealthResponse {
    HealthResponse {
        status: if state.is_ready().await {
            HealthStatus::Ok
        } else {
            HealthStatus::Degraded
        },
        service: "assistant-server".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
        server_time: OffsetDateTime::now_utc(),
    }
}
