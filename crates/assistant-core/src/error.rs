//! Structured core errors.
//!
//! Two rules govern this type. First, every variant that a user could plausibly
//! see carries a message safe to show them; provider internals stay in the
//! `#[source]` chain for logs. Second, `code()` is a stable machine-readable
//! discriminant, so the transport layer can map a failure to a wire frame
//! without matching on a growing enum in three places.

use assistant_tools::RiskLevel;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("input was empty or contained no usable content")]
    InvalidInput,

    #[error("could not assemble context for this turn")]
    ContextError(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// The provider failed. The provider's own error is preserved as the source
    /// for logging; the `Display` here says nothing about which vendor it was.
    #[error("the language model could not complete this turn")]
    ModelError(#[source] assistant_models::ModelError),

    /// No provider was injected. Distinct from `ModelError`: this is a
    /// deployment state, not a failure, and the user-facing text says so.
    #[error("no model provider is configured for this deployment")]
    NoModelProvider,

    #[error("the model proposed `{0}`, which is not a registered tool")]
    UnknownTool(String),

    #[error("arguments proposed for `{name}` were not valid: {reason}")]
    ToolValidationError { name: String, reason: String },

    #[error("`{name}` is not permitted: {reason}")]
    PermissionDenied { name: String, reason: String },

    /// Not a failure. The turn stopped deliberately and is resumable once a
    /// human decides. Modelled as an error variant so that no code path can
    /// continue past it by forgetting to check a boolean.
    #[error("`{name}` requires your approval before it can run")]
    ApprovalRequired {
        name: String,
        risk: RiskLevel,
        reason: String,
    },

    #[error("`{name}` failed while running")]
    ToolExecutionError {
        name: String,
        #[source]
        source: assistant_tools::ToolError,
    },

    #[error("the turn was cancelled")]
    Cancelled,

    #[error("the assistant reached its limit of {limit} tool rounds without finishing")]
    IterationLimitExceeded { limit: usize },

    #[error("internal error")]
    Internal(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl CoreError {
    /// Stable discriminant for the wire and for metrics. Adding a variant
    /// without adding a code here is a compile error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::ContextError(_) => "context_error",
            Self::ModelError(_) => "model_error",
            Self::NoModelProvider => "no_model_provider",
            Self::UnknownTool(_) => "unknown_tool",
            Self::ToolValidationError { .. } => "tool_validation_error",
            Self::PermissionDenied { .. } => "permission_denied",
            Self::ApprovalRequired { .. } => "approval_required",
            Self::ToolExecutionError { .. } => "tool_execution_error",
            Self::Cancelled => "cancelled",
            Self::IterationLimitExceeded { .. } => "iteration_limit_exceeded",
            Self::Internal(_) => "internal",
        }
    }

    /// Whether the turn stopped for a reason the user can act on, rather than
    /// because something broke.
    pub fn is_expected_stop(&self) -> bool {
        matches!(self, Self::ApprovalRequired { .. } | Self::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_error_display_does_not_leak_provider_detail() {
        let error = CoreError::ModelError(assistant_models::ModelError::Rejected(
            "org_id 1234 exceeded quota, key sk-abc".into(),
        ));

        let shown = error.to_string();
        assert!(!shown.contains("sk-abc"), "provider detail leaked: {shown}");
        assert!(!shown.contains("1234"), "provider detail leaked: {shown}");

        // The detail is still reachable for logs.
        let source = std::error::Error::source(&error).expect("source preserved");
        assert!(source.to_string().contains("sk-abc"));
    }

    #[test]
    fn approval_and_cancellation_are_expected_stops() {
        assert!(CoreError::Cancelled.is_expected_stop());
        assert!(
            CoreError::ApprovalRequired {
                name: "gmail.send".into(),
                risk: RiskLevel::Red,
                reason: "sends mail on your behalf".into(),
            }
            .is_expected_stop()
        );
        assert!(!CoreError::InvalidInput.is_expected_stop());
    }
}
