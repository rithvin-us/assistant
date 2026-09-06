//! Wiring the orchestrator, and translating its events onto the wire.
//!
//! This module is the boundary between `assistant-core` and the transport. The
//! core knows nothing about `ServerFrame`; the Axum handler knows nothing about
//! tool loops. Everything that has to know both lives here.

use std::sync::Arc;

use assistant_core::{
    AssistantStatusHandler, DeterministicRouter, EventBus, Orchestrator, OrchestratorConfig,
    RiskBasedPolicy, ToolRegistry,
    actions::{ActionStore, ApprovalCoordinator, ApprovalPolicy},
    turn::TurnEvent,
};
use assistant_models::{ModelId, ModelProvider};
use assistant_protocol::{ApiError, PendingApproval, RiskLevel, ServerFrame};

/// Everything the orchestrator needs that is not plain configuration.
///
/// A struct rather than a growing argument list, so adding a seam later does not
/// churn every call site. The default is inert: no provider and no tools, which
/// is exactly what a production build currently has.
pub struct Dependencies {
    /// `None` is a supported deployment: the deterministic path still answers.
    pub model: Option<Arc<dyn ModelProvider>>,
    /// Empty in production. Registering a placeholder tool would advertise a
    /// capability that does not exist; real tools arrive with the integration
    /// milestones that implement them.
    pub tools: Arc<ToolRegistry>,
    /// Durable storage for approvals, executions and audit.
    ///
    /// `None` when no `DATABASE_URL` is configured. The server still runs and
    /// the turn still stops at an approval, but the action is not persisted and
    /// cannot be resumed. `/v1/health` already reports that state as degraded,
    /// and the client is told `approval_id: null` rather than being handed an
    /// id that would fail on use.
    pub store: Option<Arc<dyn ActionStore>>,
    /// How long a pending approval stays answerable.
    pub approval_policy: ApprovalPolicy,
}

impl Default for Dependencies {
    fn default() -> Self {
        Self {
            model: None,
            tools: Arc::new(ToolRegistry::new()),
            store: None,
            approval_policy: ApprovalPolicy::default(),
        }
    }
}

/// Builds the orchestrator this deployment will use.
///
/// Returns the orchestrator and, when a durable store was supplied, the
/// coordinator that resumes approved actions. Both share one [`ToolExecutor`]:
/// the resume path must run through the same executor as the in-turn path, not
/// a second one built alongside it.
pub fn build(
    deps: Dependencies,
    events: EventBus,
    max_tool_rounds: usize,
) -> (Orchestrator, Option<Arc<ApprovalCoordinator>>) {
    let Dependencies {
        model,
        tools,
        store,
        approval_policy,
    } = deps;
    let registry = tools;

    // The status handler answers from real process state, so it is wired with
    // the same registry and provider the rest of the turn would have used.
    let router = Arc::new(
        DeterministicRouter::new().with(Arc::new(AssistantStatusHandler::new(
            model.as_ref().map(|m| m.name().to_string()),
            registry.clone(),
        ))),
    );

    // Built first so the coordinator can borrow its executor.
    let orchestrator = Orchestrator::builder()
        .registry(registry)
        .policy(Arc::new(RiskBasedPolicy::new()))
        .router(router)
        .events(events.clone())
        .config(OrchestratorConfig {
            max_tool_rounds,
            model: ModelId("default".to_string()),
            system_prompt: None,
        })
        .maybe_model(model)
        .build();

    let coordinator = store.map(|store| {
        Arc::new(ApprovalCoordinator::new(
            store,
            orchestrator.executor().clone(),
            approval_policy,
            Some(events),
        ))
    });

    // Rebuilding with the coordinator attached keeps `Orchestrator`'s fields
    // private and its construction in one place.
    let orchestrator = orchestrator.with_approvals(coordinator.clone());

    (orchestrator, coordinator)
}

/// Projects a durable approval onto the wire shape the client renders.
pub fn to_pending(approval: assistant_core::actions::ApprovalRequest) -> PendingApproval {
    PendingApproval {
        approval_id: approval.id,
        summary: format!("{} awaiting your approval", approval.tool_name),
        tool_name: approval.tool_name,
        risk: risk_to_wire(approval.risk),
        reason: approval.reason,
        created_at: approval.created_at,
        expires_at: approval.expires_at,
    }
}

/// Translates one core event into the frame a client should receive.
///
/// Returns `None` for events the wire protocol does not carry. `Started`,
/// `AssistantStarted` and `ToolStarted` are deliberately not forwarded: they add
/// round trips without telling a client anything it cannot infer from the frames
/// that do arrive.
pub fn to_frame(event: TurnEvent) -> Option<ServerFrame> {
    match event {
        TurnEvent::AssistantDelta { message_id, text } => {
            Some(ServerFrame::AssistantDelta { message_id, text })
        }

        TurnEvent::ToolProposed {
            call_id,
            name,
            risk,
        } => Some(ServerFrame::ToolProposed {
            call_id,
            name,
            risk: risk_to_wire(risk),
        }),

        TurnEvent::ApprovalRequired {
            call_id,
            name,
            risk,
            reason,
            approval_id,
            summary,
        } => Some(ServerFrame::ApprovalRequired {
            call_id,
            name,
            risk: risk_to_wire(risk),
            reason,
            approval_id,
            summary,
        }),

        TurnEvent::ToolCompleted { call_id, name, ok } => {
            Some(ServerFrame::ToolCompleted { call_id, name, ok })
        }

        TurnEvent::Completed { message_id, .. } => Some(ServerFrame::TurnEnd { message_id }),

        // `message` is the core's user-safe `Display` text. Provider internals
        // stay in the core's log via the error's source chain.
        TurnEvent::Failed { code, message, .. } => Some(ServerFrame::Error(ApiError {
            code: code.to_string(),
            message,
        })),

        _ => None,
    }
}

fn risk_to_wire(risk: assistant_tools::RiskLevel) -> RiskLevel {
    match risk {
        assistant_tools::RiskLevel::Green => RiskLevel::Green,
        assistant_tools::RiskLevel::Yellow => RiskLevel::Yellow,
        assistant_tools::RiskLevel::Orange => RiskLevel::Orange,
        assistant_tools::RiskLevel::Red => RiskLevel::Red,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn a_failed_turn_becomes_an_error_frame_carrying_the_stable_code() {
        let frame = to_frame(TurnEvent::Failed {
            turn_id: Uuid::new_v4(),
            code: "approval_required",
            message: "`gmail.send` requires your approval before it can run".into(),
        })
        .expect("frame");

        match frame {
            ServerFrame::Error(error) => {
                assert_eq!(error.code, "approval_required");
                assert!(error.message.contains("gmail.send"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn risk_crosses_the_boundary_without_being_reclassified() {
        for (core, wire) in [
            (assistant_tools::RiskLevel::Green, RiskLevel::Green),
            (assistant_tools::RiskLevel::Yellow, RiskLevel::Yellow),
            (assistant_tools::RiskLevel::Orange, RiskLevel::Orange),
            (assistant_tools::RiskLevel::Red, RiskLevel::Red),
        ] {
            assert_eq!(risk_to_wire(core), wire);
        }
    }

    #[test]
    fn lifecycle_events_the_wire_does_not_carry_are_dropped() {
        let dropped = to_frame(TurnEvent::AssistantStarted {
            message_id: Uuid::new_v4(),
        });
        assert!(dropped.is_none());
    }
}
