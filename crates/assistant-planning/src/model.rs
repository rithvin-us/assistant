//! Strongly typed Rust domain models for M9 Unified Personal Planning Engine.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub type UserId = Uuid;
pub type PlanningItemId = Uuid;

/// Source provenance of a planning item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemSource {
    StandaloneTask,
    GoogleClassroom,
    GoogleCalendar,
    Gmail,
    DocumentDeadline,
    MemoryContext,
    ProjectTask,
}

impl ItemSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StandaloneTask => "standalone_task",
            Self::GoogleClassroom => "google_classroom",
            Self::GoogleCalendar => "google_calendar",
            Self::Gmail => "gmail",
            Self::DocumentDeadline => "document_deadline",
            Self::MemoryContext => "memory_context",
            Self::ProjectTask => "project_task",
        }
    }
}

/// Hard vs Soft deadline distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadlineKind {
    /// Strict submission or appointment cutoff. Cannot be missed without consequence.
    Hard,
    /// Preferred target date, but flexible if higher-priority work conflicts.
    Soft,
}

/// Time precision of a deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadlinePrecision {
    ExactDateTime,
    DateOnly,
    PartialDate,
}

/// Structured deadline representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deadline {
    #[serde(with = "time::serde::rfc3339")]
    pub due_at: OffsetDateTime,
    pub kind: DeadlineKind,
    pub precision: DeadlinePrecision,
    pub provenance: String,
    pub confidence: Option<f32>,
}

/// Estimated work effort duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffortEstimate {
    /// Explicit known estimate in minutes.
    Known { minutes: u32 },
    /// No reliable estimate exists. Must NOT be defaulted to an arbitrary number.
    Unknown,
}

impl EffortEstimate {
    pub fn known_minutes(self) -> Option<u32> {
        match self {
            Self::Known { minutes } => Some(minutes),
            Self::Unknown => None,
        }
    }
}

/// Priority levels matching system semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningPriority {
    Low = 1,
    Medium = 2,
    High = 3,
    Urgent = 4,
}

impl PlanningPriority {
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "urgent" | "p1" => Self::Urgent,
            "high" | "p2" => Self::High,
            "medium" | "p3" => Self::Medium,
            _ => Self::Low,
        }
    }
}

/// An item requiring user time or action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningItem {
    pub id: PlanningItemId,
    pub user_id: UserId,
    pub title: String,
    pub description: Option<String>,
    pub source: ItemSource,
    pub source_ref: Option<String>,
    pub priority: PlanningPriority,
    pub deadline: Option<Deadline>,
    pub effort: EffortEstimate,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    pub is_completed: bool,
    pub dependencies: Vec<PlanningItemId>,
}

/// An obligation occupying non-plannable time (meeting, class, appointment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: String,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub is_all_day: bool,
    pub location: Option<String>,
    pub source: ItemSource,
}

/// Usable time window available for scheduled work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailabilityWindow {
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub duration_minutes: u32,
    pub is_usable: bool,
    pub source: String,
}

/// A proposed allocated work block scheduled into an available window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanBlock {
    pub item_id: PlanningItemId,
    pub title: String,
    #[serde(with = "time::serde::rfc3339")]
    pub start_time: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub end_time: OffsetDateTime,
    pub duration_minutes: u32,
    pub rationale: String,
    pub confidence: f32,
    pub source: ItemSource,
}

/// Type of schedule or deadline conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    OverlappingCommitments,
    InsufficientTimeBeforeDeadline,
    HardDeadlinesCompeting,
    DependencyBlocked,
    ScheduledPastDeadline,
}

/// Severity rating for conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictSeverity {
    Warning,
    Critical,
}

/// Details of a detected planning conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub conflict_type: ConflictType,
    pub severity: ConflictSeverity,
    pub affected_item_ids: Vec<PlanningItemId>,
    pub reason: String,
}

/// Overall feasibility classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeasibilityState {
    Feasible,
    LikelyFeasible,
    Uncertain,
    Infeasible,
}

/// Detailed feasibility calculation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeasibilityResult {
    pub state: FeasibilityState,
    pub total_known_effort_minutes: u32,
    pub total_available_minutes: u32,
    pub unknown_effort_count: usize,
    pub conflicts: Vec<Conflict>,
    pub explanation: String,
}

/// Today's generated planning view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayPlan {
    #[serde(with = "time::serde::rfc3339")]
    pub date: OffsetDateTime,
    pub commitments: Vec<Commitment>,
    pub recommended_blocks: Vec<PlanBlock>,
    pub upcoming_deadlines: Vec<Deadline>,
    pub total_available_minutes: u32,
    pub conflicts: Vec<Conflict>,
    pub feasibility: FeasibilityResult,
}

/// Multi-day planning outlook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpcomingPlanning {
    pub horizon_days: u32,
    pub total_items: usize,
    pub deadlines: Vec<PlanningItem>,
    pub daily_workload_minutes: Vec<(String, u32)>,
    pub conflicts: Vec<Conflict>,
    pub feasibility: FeasibilityResult,
}
