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

/// Implemented once per tool.
///
/// `execute` is only ever reached after policy has returned
/// [`PermissionDecision::Allow`]; a tool must not evaluate its own permissions.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> &ToolSpec;

    async fn execute(&self, args: serde_json::Value) -> Result<serde_json::Value, ToolError>;
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
}
