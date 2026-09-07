//! Tool implementations for Unified Personal Planning (M9).

use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{RiskLevel, Tool, ToolError, ToolSpec};

/// Trait implemented by the server planning aggregator to provide structured planning computations.
#[async_trait]
pub trait PlanningProvider: Send + Sync {
    async fn get_today_plan(&self, user_id: Uuid) -> Result<serde_json::Value, ToolError>;
    async fn get_upcoming_planning(
        &self,
        user_id: Uuid,
        horizon_days: u32,
    ) -> Result<serde_json::Value, ToolError>;
    async fn analyze_schedule(
        &self,
        user_id: Uuid,
        horizon_days: u32,
    ) -> Result<serde_json::Value, ToolError>;
    async fn check_feasibility(&self, user_id: Uuid) -> Result<serde_json::Value, ToolError>;
    async fn detect_conflicts(&self, user_id: Uuid) -> Result<serde_json::Value, ToolError>;
}

fn user_id(args: &serde_json::Value) -> Result<Uuid, ToolError> {
    let raw = args
        .get("_user_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::Failed("no authenticated user in context".into()))?;
    Uuid::parse_str(raw).map_err(|_| ToolError::Failed("invalid user context".into()))
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

pub struct GetTodayPlanTool {
    spec: ToolSpec,
    provider: Arc<dyn PlanningProvider>,
}

impl GetTodayPlanTool {
    pub fn new(provider: Arc<dyn PlanningProvider>) -> Self {
        Self {
            spec: ToolSpec {
                name: "planning.today".to_string(),
                description: "Gets the user's unified today plan, including commitments, recommended focus work blocks, available time, and conflicts.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
            provider,
        }
    }
}

#[async_trait]
impl Tool for GetTodayPlanTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let uid = user_id(&args)?;
        self.provider.get_today_plan(uid).await
    }
}

pub struct GetUpcomingDeadlinesTool {
    spec: ToolSpec,
    provider: Arc<dyn PlanningProvider>,
}

impl GetUpcomingDeadlinesTool {
    pub fn new(provider: Arc<dyn PlanningProvider>) -> Self {
        Self {
            spec: ToolSpec {
                name: "planning.upcoming".to_string(),
                description: "Gets upcoming deadlines, workload forecast, and multi-day planning feasibility.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "horizon_days": { "type": "integer", "description": "Number of days to look ahead (default 7)" }
                    },
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
            provider,
        }
    }
}

#[async_trait]
impl Tool for GetUpcomingDeadlinesTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let uid = user_id(&args)?;
        let horizon = args
            .get("horizon_days")
            .and_then(|v| v.as_u64())
            .unwrap_or(7) as u32;
        self.provider.get_upcoming_planning(uid, horizon).await
    }
}

pub struct AnalyzeScheduleTool {
    spec: ToolSpec,
    provider: Arc<dyn PlanningProvider>,
}

impl AnalyzeScheduleTool {
    pub fn new(provider: Arc<dyn PlanningProvider>) -> Self {
        Self {
            spec: ToolSpec {
                name: "planning.analyze".to_string(),
                description: "Analyzes schedule feasibility, total work effort, available time, and potential conflicts.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "horizon_days": { "type": "integer", "description": "Number of days to analyze (default 7)" }
                    },
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
            provider,
        }
    }
}

#[async_trait]
impl Tool for AnalyzeScheduleTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let uid = user_id(&args)?;
        let horizon = args
            .get("horizon_days")
            .and_then(|v| v.as_u64())
            .unwrap_or(7) as u32;
        self.provider.analyze_schedule(uid, horizon).await
    }
}

pub struct CheckFeasibilityTool {
    spec: ToolSpec,
    provider: Arc<dyn PlanningProvider>,
}

impl CheckFeasibilityTool {
    pub fn new(provider: Arc<dyn PlanningProvider>) -> Self {
        Self {
            spec: ToolSpec {
                name: "planning.check_feasibility".to_string(),
                description: "Checks whether the user's current tasks and commitments are realistic to finish before deadlines.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
            provider,
        }
    }
}

#[async_trait]
impl Tool for CheckFeasibilityTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let uid = user_id(&args)?;
        self.provider.check_feasibility(uid).await
    }
}

pub struct DetectConflictsTool {
    spec: ToolSpec,
    provider: Arc<dyn PlanningProvider>,
}

impl DetectConflictsTool {
    pub fn new(provider: Arc<dyn PlanningProvider>) -> Self {
        Self {
            spec: ToolSpec {
                name: "planning.detect_conflicts".to_string(),
                description: "Detects scheduling conflicts, overlapping meetings, hard deadline collisions, or unestimated workload risks.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
                risk: RiskLevel::Green,
                required_scopes: vec![],
                timeout_ms: 10_000,
            },
            provider,
        }
    }
}

#[async_trait]
impl Tool for DetectConflictsTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let uid = user_id(&args)?;
        self.provider.detect_conflicts(uid).await
    }
}
