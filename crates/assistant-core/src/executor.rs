//! Tool execution.
//!
//! One path, no shortcuts: resolve against the registry, read the authoritative
//! spec, ask policy, then -- only on `Allow` -- run the tool under its declared
//! timeout and a cancellation token.
//!
//! There is no second entry point that skips policy. `execute` is the only
//! public way to run a tool, and it always calls [`PermissionPolicy::evaluate`]
//! first.

use std::{sync::Arc, time::Duration};

use assistant_auth::Principal;
use assistant_tools::{PermissionDecision, ToolCall, ToolError, ToolResult, ToolSpec};
use tokio_util::sync::CancellationToken;

use crate::{CoreError, permission::PermissionPolicy, registry::ToolRegistry};

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    policy: Arc<dyn PermissionPolicy>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>, policy: Arc<dyn PermissionPolicy>) -> Self {
        Self { registry, policy }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Looks up the authoritative spec for a proposed call.
    ///
    /// A call naming a tool that is not registered can never run: there is no
    /// spec to authorise it against, so it fails here rather than reaching
    /// policy or execution.
    pub fn resolve(&self, call: &ToolCall) -> Result<ToolSpec, CoreError> {
        self.registry
            .spec(&call.name)
            .ok_or_else(|| CoreError::UnknownTool(call.name.clone()))
    }

    /// Evaluates policy for a proposed call.
    ///
    /// Separated from [`Self::execute`] so the orchestrator can decide the whole
    /// batch's fate -- and emit an approval event -- before anything runs.
    pub fn decide(&self, spec: &ToolSpec, principal: &Principal) -> PermissionDecision {
        self.policy.evaluate(spec, principal)
    }

    /// Resolves, authorises and runs one tool call.
    ///
    /// A tool that returns an error yields an `Err` variant inside a
    /// [`ToolResult`], not a panic and not a failed turn: a tool failing is
    /// ordinary, and the model is usually able to recover from being told so.
    /// Only conditions that must stop the turn -- unknown tool, denial, pending
    /// approval, cancellation -- surface as [`CoreError`].
    #[tracing::instrument(
        skip_all,
        fields(tool = %call.name, call_id = %call.id, risk = tracing::field::Empty)
    )]
    pub async fn execute(
        &self,
        call: &ToolCall,
        principal: &Principal,
        cancel: &CancellationToken,
    ) -> Result<ToolResult, CoreError> {
        let spec = self.resolve(call)?;
        tracing::Span::current().record("risk", tracing::field::debug(spec.risk));

        // Structural validation, repeated here rather than trusted from the
        // caller: `execute` is the only way to run a tool, so the check belongs
        // where it cannot be skipped. Full JSON Schema validation against
        // `spec.input_schema` is separate, later work.
        if !call.arguments.is_object() {
            return Err(CoreError::ToolValidationError {
                name: spec.name.clone(),
                reason: "arguments must be a JSON object".to_string(),
            });
        }

        match self.decide(&spec, principal) {
            PermissionDecision::Allow => {}
            PermissionDecision::Deny { reason } => {
                tracing::warn!(decision = "deny", "tool call refused by policy");
                return Err(CoreError::PermissionDenied {
                    name: spec.name.clone(),
                    reason,
                });
            }
            PermissionDecision::RequireApproval { reason } => {
                tracing::info!(decision = "require_approval", "tool call held for approval");
                return Err(CoreError::ApprovalRequired {
                    name: spec.name.clone(),
                    risk: spec.risk,
                    reason,
                });
            }
        }

        self.run_authorized_with_user(call, &spec, Some(principal.user_id), cancel)
            .await
    }

    /// Runs a call that has **already** been authorised.
    ///
    /// This delegates to [`Self::run_authorized_with_user`] without an explicit user.
    pub async fn run_authorized(
        &self,
        call: &ToolCall,
        spec: &ToolSpec,
        cancel: &CancellationToken,
    ) -> Result<ToolResult, CoreError> {
        self.run_authorized_with_user(call, spec, None, cancel)
            .await
    }

    /// Runs a call that has already been authorised, with caller's authenticated user ID.
    pub async fn run_authorized_with_user(
        &self,
        call: &ToolCall,
        spec: &ToolSpec,
        user_id: Option<uuid::Uuid>,
        cancel: &CancellationToken,
    ) -> Result<ToolResult, CoreError> {
        let tool = self
            .registry
            .get(&call.name)
            .ok_or_else(|| CoreError::UnknownTool(call.name.clone()))?;

        // The timeout comes from the spec, so a tool cannot extend its own
        // deadline. Cancellation wins over both.
        let timeout = Duration::from_millis(spec.timeout_ms);
        let arguments = call.arguments.clone();

        let outcome = tokio::select! {
            biased;

            () = cancel.cancelled() => return Err(CoreError::Cancelled),

            result = tokio::time::timeout(timeout, tool.execute_with_user(user_id, arguments)) => result,
        };

        let result = match outcome {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                // The tool's own message is shown to the model so it can adapt;
                // it must therefore never contain credentials. That obligation
                // sits with the tool, and is stated in the `Tool` contract.
                tracing::warn!(error = %error, "tool returned an error");
                Err(error.to_string())
            }
            Err(_elapsed) => {
                tracing::warn!(timeout_ms = spec.timeout_ms, "tool timed out");
                Err(ToolError::Timeout {
                    name: spec.name.clone(),
                    timeout_ms: spec.timeout_ms,
                }
                .to_string())
            }
        };

        Ok(ToolResult {
            call_id: call.id.clone(),
            name: call.name.clone(),
            result,
        })
    }
}

impl std::fmt::Debug for ToolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolExecutor")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{permission::RiskBasedPolicy, testing::*};
    use assistant_tools::RiskLevel;

    fn executor_with(tools: Vec<Arc<dyn assistant_tools::Tool>>) -> ToolExecutor {
        let mut registry = ToolRegistry::new();
        for tool in tools {
            registry.register(tool);
        }
        ToolExecutor::new(Arc::new(registry), Arc::new(RiskBasedPolicy::new()))
    }

    #[tokio::test]
    async fn an_allowed_tool_runs_and_returns_its_value() {
        let executor = executor_with(vec![Arc::new(EchoTool::green("notes.read"))]);
        let call = call("notes.read", serde_json::json!({"q": "hi"}));

        let result = executor
            .execute(&call, &dev_principal(), &CancellationToken::new())
            .await
            .expect("executed");

        assert_eq!(result.name, "notes.read");
        assert!(result.result.is_ok());
    }

    #[tokio::test]
    async fn an_unknown_tool_never_runs() {
        let executor = executor_with(vec![]);
        let error = executor
            .execute(
                &call("gmail.send", serde_json::json!({})),
                &dev_principal(),
                &CancellationToken::new(),
            )
            .await
            .expect_err("refused");

        assert_eq!(error.code(), "unknown_tool");
    }

    #[tokio::test]
    async fn an_approval_required_tool_does_not_execute() {
        let tool = Arc::new(EchoTool::red("gmail.send"));
        let executor = executor_with(vec![tool.clone()]);

        let error = executor
            .execute(
                &call("gmail.send", serde_json::json!({})),
                &dev_principal(),
                &CancellationToken::new(),
            )
            .await
            .expect_err("held");

        assert_eq!(error.code(), "approval_required");
        assert_eq!(
            tool.calls(),
            0,
            "a tool awaiting approval must not have been invoked"
        );
    }

    #[tokio::test]
    async fn a_denied_tool_does_not_execute() {
        let tool = Arc::new(EchoTool::green("notes.read"));
        let mut registry = ToolRegistry::new();
        registry.register(tool.clone());
        let executor = ToolExecutor::new(
            Arc::new(registry),
            Arc::new(RiskBasedPolicy::new().block("notes.read")),
        );

        let error = executor
            .execute(
                &call("notes.read", serde_json::json!({})),
                &dev_principal(),
                &CancellationToken::new(),
            )
            .await
            .expect_err("denied");

        assert_eq!(error.code(), "permission_denied");
        assert_eq!(tool.calls(), 0, "a denied tool must not have been invoked");
    }

    #[tokio::test]
    async fn a_failing_tool_is_a_structured_result_not_a_turn_failure() {
        let executor = executor_with(vec![Arc::new(FailingTool::new("notes.read"))]);

        let result = executor
            .execute(
                &call("notes.read", serde_json::json!({})),
                &dev_principal(),
                &CancellationToken::new(),
            )
            .await
            .expect("the turn survives a tool failure");

        assert!(result.result.is_err());
    }

    #[tokio::test]
    async fn a_slow_tool_is_stopped_at_its_declared_timeout() {
        let executor = executor_with(vec![Arc::new(SlowTool::new("notes.read", 50))]);

        let result = executor
            .execute(
                &call("notes.read", serde_json::json!({})),
                &dev_principal(),
                &CancellationToken::new(),
            )
            .await
            .expect("timeout is a result, not a panic");

        match result.result {
            Err(message) => assert!(message.contains("timeout"), "unexpected: {message}"),
            Ok(value) => panic!("slow tool should not have completed: {value}"),
        }
    }

    #[tokio::test]
    async fn cancellation_stops_execution() {
        let executor = executor_with(vec![Arc::new(SlowTool::new("notes.read", 10_000))]);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let error = executor
            .execute(
                &call("notes.read", serde_json::json!({})),
                &dev_principal(),
                &cancel,
            )
            .await
            .expect_err("cancelled");

        assert_eq!(error.code(), "cancelled");
    }

    #[tokio::test]
    async fn a_call_whose_arguments_are_not_an_object_is_rejected_before_the_tool() {
        let tool = Arc::new(EchoTool::green("notes.read"));
        let executor = executor_with(vec![tool.clone()]);

        let malformed = ToolCall {
            id: "call_1".into(),
            name: "notes.read".into(),
            arguments: serde_json::json!("not an object"),
        };

        let error = executor
            .execute(&malformed, &dev_principal(), &CancellationToken::new())
            .await
            .expect_err("rejected");

        assert_eq!(error.code(), "tool_validation_error");
        assert_eq!(tool.calls(), 0);
    }

    #[tokio::test]
    async fn model_supplied_arguments_cannot_change_the_risk_decision() {
        // The call's arguments claim the tool is harmless. The registry says Red.
        let tool = Arc::new(EchoTool::red("gmail.send"));
        let executor = executor_with(vec![tool.clone()]);

        let hostile = ToolCall {
            id: "call_1".into(),
            name: "gmail.send".into(),
            arguments: serde_json::json!({
                "risk": "green",
                "risk_level": "Green",
                "require_approval": false,
                "__policy_override": "allow"
            }),
        };

        let error = executor
            .execute(&hostile, &dev_principal(), &CancellationToken::new())
            .await
            .expect_err("still held");

        assert_eq!(error.code(), "approval_required");
        assert_eq!(tool.calls(), 0);
        assert_eq!(
            executor.resolve(&hostile).expect("spec").risk,
            RiskLevel::Red
        );
    }
}
