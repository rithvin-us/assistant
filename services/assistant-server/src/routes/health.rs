use assistant_protocol::{HealthResponse, HealthStatus, PROTOCOL_VERSION};
use axum::{Json, extract::State};
use time::OffsetDateTime;

use crate::state::SharedState;

/// Public liveness and capability probe.
///
/// Reports `Degraded` rather than failing when the database is absent, so the
/// mobile app can distinguish "server unreachable" from "server up, storage not
/// configured".
pub async fn health(State(state): State<SharedState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: if state.is_healthy() {
            HealthStatus::Ok
        } else {
            HealthStatus::Degraded
        },
        service: "assistant-server".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
        server_time: OffsetDateTime::now_utc(),
    })
}
