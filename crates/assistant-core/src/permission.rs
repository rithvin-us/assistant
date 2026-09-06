//! Deterministic permission evaluation.
//!
//! The signature is the security property. `evaluate` takes a [`ToolSpec`] --
//! which only the registry can produce -- and a [`Principal`] -- which only the
//! authentication layer can produce. It does not take a `ToolCall`, so there is
//! no parameter through which model output could reach a decision, and no
//! amount of prompt injection can change what this function returns.
//!
//! See ADR-0005.

use std::collections::HashSet;

use assistant_auth::Principal;
use assistant_tools::{PermissionDecision, RiskLevel, ToolSpec};

pub trait PermissionPolicy: Send + Sync {
    /// Decides whether a tool may run for this caller.
    ///
    /// Must be pure and fast: it is on the critical path of every tool call, and
    /// a decision that depends on anything mutable is a decision that cannot be
    /// audited after the fact.
    fn evaluate(&self, spec: &ToolSpec, principal: &Principal) -> PermissionDecision;
}

/// The default policy: risk class plus scope check.
///
/// Deliberately boring. Every rule is a comparison against a value fixed in Rust
/// at the tool's declaration site.
#[derive(Debug, Clone)]
pub struct RiskBasedPolicy {
    /// Risk at or above which a human must approve. Defaults to `Orange`.
    approval_at_or_above: RiskLevel,
    /// Tools refused outright regardless of risk or scope.
    blocked: HashSet<String>,
    /// When true, a principal is not required to hold a tool's declared scopes.
    ///
    /// Only for local development, where the placeholder verifier issues a
    /// single `dev` scope. It weakens a real control, so it is off by default
    /// and named so that enabling it is visible in review.
    allow_missing_scopes: bool,
}

impl Default for RiskBasedPolicy {
    fn default() -> Self {
        Self {
            approval_at_or_above: RiskLevel::Orange,
            blocked: HashSet::new(),
            allow_missing_scopes: false,
        }
    }
}

impl RiskBasedPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn approval_at_or_above(mut self, risk: RiskLevel) -> Self {
        self.approval_at_or_above = risk;
        self
    }

    pub fn block(mut self, name: impl Into<String>) -> Self {
        self.blocked.insert(name.into());
        self
    }

    /// See [`Self::allow_missing_scopes`]. Development only.
    pub fn allowing_missing_scopes(mut self) -> Self {
        self.allow_missing_scopes = true;
        self
    }

    fn missing_scopes(&self, spec: &ToolSpec, principal: &Principal) -> Vec<String> {
        if self.allow_missing_scopes {
            return Vec::new();
        }
        spec.required_scopes
            .iter()
            .filter(|scope| !principal.scopes.contains(scope))
            .cloned()
            .collect()
    }
}

impl PermissionPolicy for RiskBasedPolicy {
    fn evaluate(&self, spec: &ToolSpec, principal: &Principal) -> PermissionDecision {
        // Order matters. A blocklist entry wins over everything, and a scope
        // failure is a denial rather than an approval prompt: asking a user to
        // approve an action their credentials cannot perform would be theatre.
        if self.blocked.contains(&spec.name) {
            return PermissionDecision::Deny {
                reason: format!("`{}` is blocked by policy", spec.name),
            };
        }

        let missing = self.missing_scopes(spec, principal);
        if !missing.is_empty() {
            return PermissionDecision::Deny {
                reason: format!(
                    "your account has not granted the required access: {}",
                    missing.join(", ")
                ),
            };
        }

        if spec.risk >= self.approval_at_or_above {
            return PermissionDecision::RequireApproval {
                reason: format!(
                    "`{}` is classified {:?} and needs your confirmation",
                    spec.name, spec.risk
                ),
            };
        }

        PermissionDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::spec_with;

    fn principal_with(scopes: &[&str]) -> Principal {
        Principal {
            user_id: assistant_auth::DevTokenVerifier::DEV_USER_ID,
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn read_only_tools_are_allowed() {
        let policy = RiskBasedPolicy::new();
        let spec = spec_with("notes.read", RiskLevel::Green, &[]);
        assert_eq!(
            policy.evaluate(&spec, &principal_with(&[])),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn reversible_organisation_is_allowed() {
        let policy = RiskBasedPolicy::new();
        let spec = spec_with("tasks.create", RiskLevel::Yellow, &[]);
        assert_eq!(
            policy.evaluate(&spec, &principal_with(&[])),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn consequential_and_destructive_tools_require_approval() {
        let policy = RiskBasedPolicy::new();

        for risk in [RiskLevel::Orange, RiskLevel::Red] {
            let spec = spec_with("gmail.send", risk, &[]);
            assert!(
                matches!(
                    policy.evaluate(&spec, &principal_with(&[])),
                    PermissionDecision::RequireApproval { .. }
                ),
                "{risk:?} should require approval"
            );
        }
    }

    #[test]
    fn a_missing_scope_denies_rather_than_prompting() {
        let policy = RiskBasedPolicy::new();
        let spec = spec_with("gmail.send", RiskLevel::Red, &["gmail.send"]);

        let decision = policy.evaluate(&spec, &principal_with(&["calendar.read"]));
        match decision {
            PermissionDecision::Deny { reason } => assert!(reason.contains("gmail.send")),
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn a_blocked_tool_is_denied_even_when_it_would_otherwise_be_allowed() {
        let policy = RiskBasedPolicy::new().block("notes.read");
        let spec = spec_with("notes.read", RiskLevel::Green, &[]);

        assert!(matches!(
            policy.evaluate(&spec, &principal_with(&[])),
            PermissionDecision::Deny { .. }
        ));
    }

    /// The central invariant of ADR-0005.
    ///
    /// `evaluate` has no parameter that model output can reach. The only risk
    /// value it can read is the one on the registry's `ToolSpec`, so two specs
    /// that differ only in risk must produce different decisions for the same
    /// tool name -- proving the decision follows the declaration, not the name
    /// or anything a model could say about it.
    #[test]
    fn the_decision_follows_the_registered_risk_not_the_tool_name() {
        let policy = RiskBasedPolicy::new();
        let principal = principal_with(&[]);

        let declared_safe = spec_with("gmail.send", RiskLevel::Green, &[]);
        let declared_dangerous = spec_with("gmail.send", RiskLevel::Red, &[]);

        assert_eq!(
            policy.evaluate(&declared_safe, &principal),
            PermissionDecision::Allow
        );
        assert!(matches!(
            policy.evaluate(&declared_dangerous, &principal),
            PermissionDecision::RequireApproval { .. }
        ));
    }
}
