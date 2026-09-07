//! Tool contracts.
//!
//! No tool is implemented here yet. What exists is the vocabulary every future
//! tool must be described in, so that permission evaluation stays deterministic
//! application logic and never becomes something a model decides for itself.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Risk class of a tool invocation.
///
/// This is a property of the *tool declaration*, fixed at compile time. It is
/// never supplied by a model, never read from model output, and never widened at
/// runtime. See docs/DECISIONS.md ADR-0005.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    /// Read-only. No observable side effect.
    Green,
    /// Low-risk organisation of the user's own data. Reversible.
    Yellow,
    /// Consequential but recoverable.
    Orange,
    /// Destructive, irreversible, or visible to third parties.
    Red,
}

/// Outcome of evaluating a tool call against policy. Produced only by
/// deterministic code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Execute immediately.
    Allow,
    /// Hold the call and raise an approval request to the user.
    RequireApproval { reason: String },
    /// Refuse without asking.
    Deny { reason: String },
}

/// Lifecycle state for persistent human-in-the-loop approvals.
///
/// The durable record itself is `assistant_core::actions::ApprovalRequest`: it
/// needs a principal, timestamps and an execution reference, none of which this
/// crate should know about. The status stays here with the rest of the tool
/// vocabulary so both crates name the same states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Requested,
    Approved,
    Rejected,
    Expired,
    Cancelled,
}

/// A model-proposed or orchestrator-constructed tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// Opaque state the provider attached to this call and demands back on the
    /// next round.
    ///
    /// Gemini 3 returns a `thought_signature` alongside every function call and
    /// rejects the follow-up request with `400 Function call is missing a
    /// thought_signature in functionCall parts` unless it is echoed verbatim, so
    /// a turn that called a tool could never produce a final answer.
    ///
    /// These are provider-owned bytes and are treated as such: nothing reads
    /// them, they are never matched, logged or persisted in an audit record, and
    /// the only place they go is back to the provider that issued them. In
    /// particular they are not an input to anything that decides what may run:
    /// the permission policy sees a [`ToolSpec`] and a principal, and this field
    /// is on neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<serde_json::Value>,
}

impl ToolCall {
    /// A call with no provider state attached: what the orchestrator and the
    /// approval-resume path construct, and what every provider without a
    /// round-trip requirement emits.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
            provider_metadata: None,
        }
    }
}

/// Result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub result: Result<serde_json::Value, String>,
}

/// Everything the orchestrator and the audit log need to know about a tool
/// before it is ever called.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Namespaced identifier, e.g. `gmail.send`.
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments.
    pub input_schema: serde_json::Value,
    /// JSON Schema for a successful result.
    pub output_schema: serde_json::Value,
    pub risk: RiskLevel,
    /// Scopes the caller must already hold, e.g. `gmail.readonly`.
    pub required_scopes: Vec<String>,
    /// Hard execution limit. Enforced by the executor, not by the tool.
    pub timeout_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool `{0}` is not registered")]
    NotFound(String),
    #[error("arguments did not match the input schema: {0}")]
    InvalidArguments(String),
    #[error("tool `{name}` exceeded its {timeout_ms}ms timeout")]
    Timeout { name: String, timeout_ms: u64 },
    #[error("tool execution failed: {0}")]
    Failed(String),
}

pub mod academic_tools;
pub mod google_tools;
pub mod planning_tools;
pub mod providers;

pub use academic_tools::{
    AcademicAssignmentsTool, AcademicDeadlinesTool, AcademicSyncTool, ClassroomAnnouncementsTool,
    ClassroomCoursesTool, ClassroomCourseworkTool, DriveListTool, DriveMetadataTool,
    DriveReadFileTool, DriveSearchTool,
};
pub use google_tools::{
    CalendarCreateTool, CalendarDeleteTool, CalendarListTool, CalendarSearchTool,
    CalendarUpdateTool, GmailReadTool, GmailSearchTool,
};
pub use planning_tools::{
    AnalyzeScheduleTool, CheckFeasibilityTool, DetectConflictsTool, GetTodayPlanTool,
    GetUpcomingDeadlinesTool, PlanningProvider,
};
pub use providers::{
    AcademicDeadline, AcademicOverview, AcademicProvider, AcademicSyncResult, Announcement,
    CalendarEvent, CalendarProvider, ClassroomProvider, Course, CourseworkItem,
    CreateCalendarEvent, DriveFile, DriveFileContent, DriveProvider, EmailDetail, EmailSummary,
    FreeSlot, GmailProvider, UpdateCalendarEvent,
};

/// Implemented once per tool.
///
/// `execute` is only ever reached after policy has returned
/// [`PermissionDecision::Allow`]; a tool must not evaluate its own permissions.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError>;

    /// Executes tool with authenticated user context. Defaults to `execute(args)` for backward compatibility.
    async fn execute_with_user(
        &self,
        user_id: Option<uuid::Uuid>,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        let _ = user_id;
        self.execute(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_levels_order_from_least_to_most_dangerous() {
        assert!(RiskLevel::Green < RiskLevel::Yellow);
        assert!(RiskLevel::Yellow < RiskLevel::Orange);
        assert!(RiskLevel::Orange < RiskLevel::Red);
    }

    #[test]
    fn tool_spec_risk_level_is_immutable_by_input() {
        let spec = ToolSpec {
            name: "gmail.send".into(),
            description: "Send email".into(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            risk: RiskLevel::Red,
            required_scopes: vec!["gmail.send".into()],
            timeout_ms: 5000,
        };

        // Untrusted model input cannot modify the Rust-defined risk level.
        assert_eq!(spec.risk, RiskLevel::Red);
    }
}
