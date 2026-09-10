//! REST endpoints for M9 Unified Personal Planning Engine.

use assistant_auth::Principal;
use assistant_protocol::{PlanGenerationRequest, PlanningAnalysisRequest};
use assistant_tools::PlanningProvider;
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::Deserialize;

use crate::{error::AppError, planning::ServerPlanningAggregator, state::SharedState};

#[derive(Debug, Deserialize)]
pub struct HorizonQuery {
    pub days: Option<u32>,
}

fn get_aggregator(state: &SharedState) -> Result<ServerPlanningAggregator, AppError> {
    let pool = state
        .db
        .as_ref()
        .ok_or_else(|| AppError::DependencyUnavailable("the database"))?;
    Ok(ServerPlanningAggregator::new(pool.clone()))
}

/// GET `/v1/planning/today`
pub async fn get_today_plan(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, AppError> {
    let aggregator = get_aggregator(&state)?;
    let res = aggregator
        .get_today_plan(principal.user_id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    Ok(Json(res))
}

/// GET `/v1/planning/upcoming`
pub async fn get_upcoming_planning(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<HorizonQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let aggregator = get_aggregator(&state)?;
    let horizon = query.days.unwrap_or(7);
    let res = aggregator
        .get_upcoming_planning(principal.user_id, horizon)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    Ok(Json(res))
}

/// POST `/v1/planning/analyze`
pub async fn analyze_planning(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(payload): Json<PlanningAnalysisRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let aggregator = get_aggregator(&state)?;
    let horizon = payload.horizon_days.unwrap_or(7);
    let res = aggregator
        .analyze_schedule(principal.user_id, horizon)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    Ok(Json(res))
}

/// POST `/v1/planning/plan`
pub async fn generate_plan(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(_payload): Json<PlanGenerationRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let aggregator = get_aggregator(&state)?;
    let res = aggregator
        .get_today_plan(principal.user_id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    Ok(Json(res))
}

/// GET `/v1/planning/conflicts`
pub async fn get_conflicts(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, AppError> {
    let aggregator = get_aggregator(&state)?;
    let res = aggregator
        .detect_conflicts(principal.user_id)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;

    Ok(Json(res))
}
