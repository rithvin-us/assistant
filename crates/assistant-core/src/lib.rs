//! Assistant orchestration: the execution spine.
//!
//! This crate owns the pipeline that turns user input into an answer:
//! normalisation, context assembly, execution routing, the model call, the
//! bounded tool loop, permission evaluation and streaming.
//!
//! Two independences are load-bearing and are enforced by this crate's
//! dependency list rather than by convention:
//!
//! * **Provider independence.** Nothing here names Claude, Gemini or any other
//!   vendor. Models arrive as [`assistant_models::ModelProvider`] implementations.
//! * **Transport independence.** Nothing here knows about HTTP, WebSockets,
//!   Axum or Tauri. Output is a stream of [`turn::TurnEvent`]s that a transport
//!   translates into its own wire format.
//!
//! Integrations (Gmail, Calendar, Drive, browser, desktop) will arrive as
//! [`assistant_tools::Tool`] implementations registered in a [`registry::ToolRegistry`].
//! None of them requires a change here.

pub mod context;
pub mod deterministic;
pub mod error;
pub mod event;
pub mod executor;
pub mod orchestrator;
pub mod permission;
pub mod registry;
pub mod turn;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use context::{ContextProvider, EmptyContextProvider, InMemoryContextProvider};
pub use deterministic::{AssistantStatusHandler, DeterministicHandler, DeterministicRouter};
pub use error::CoreError;
pub use event::{DomainEvent, EventBus, EventEnvelope};
pub use executor::ToolExecutor;
pub use orchestrator::{Orchestrator, OrchestratorBuilder, OrchestratorConfig};
pub use permission::{PermissionPolicy, RiskBasedPolicy};
pub use registry::ToolRegistry;
pub use turn::{
    ExecutionMode, ExecutionPlan, NormalizedInput, TurnContext, TurnEvent, TurnOutcome,
    TurnRequest, normalize,
};
