//! The orchestrator.
//!
//! One turn, start to finish:
//!
//! ```text
//! input -> normalize -> context -> route
//!            |                       |
//!            |            deterministic? -> handler -> answer   (no model call)
//!            |                       |
//!            |                     model -> stream deltas
//!            |                              |
//!            |                        tool calls? -> registry -> policy
//!            |                                          |
//!            |                              Allow -> execute -> feed back -> model
//!            |                              RequireApproval -> stop
//!            |                              Deny -> stop
//!            |                                          |
//!            +--------------------------------> final answer
//! ```
//!
//! Every dependency is injected. The orchestrator constructs no providers, reads
//! no environment variables and holds no global state, so a test can drive the
//! real code path with fakes and the server can drive it with whatever is
//! configured.

use std::sync::Arc;

use assistant_auth::Principal;
use assistant_models::{GenerateRequest, Message, ModelId, ModelProvider, StreamChunk};
use assistant_tools::{PermissionDecision, RiskLevel, ToolCall, ToolResult};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    CoreError,
    actions::{ApprovalCoordinator, summarize},
    context::{ContextProvider, EmptyContextProvider},
    deterministic::{DeterministicHandler, DeterministicRouter},
    event::{DomainEvent, EventBus},
    executor::ToolExecutor,
    permission::{PermissionPolicy, RiskBasedPolicy},
    registry::ToolRegistry,
    turn::{
        ContextRole, ExecutionMode, ExecutionPlan, MessageId, NormalizedInput, TurnContext,
        TurnEvent, TurnOutcome, TurnRequest, normalize,
    },
};

/// Runtime knobs the core needs. Supplied by the caller; the core never reads
/// configuration from the environment itself.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Hard ceiling on rounds of tool execution in one turn.
    ///
    /// The model can never raise this: it is read from here and nowhere else.
    /// A turn makes at most `max_tool_rounds + 1` model calls.
    pub max_tool_rounds: usize,
    /// Model the provider should use for this deployment.
    pub model: ModelId,
    pub system_prompt: Option<String>,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            // Enough for a lookup, a follow-up and a correction; low enough that
            // a confused model cannot spend real money going in circles.
            max_tool_rounds: 4,
            model: ModelId("default".to_string()),
            system_prompt: None,
        }
    }
}

/// Fan-out for [`TurnEvent`]s. `None` when the caller does not want a stream.
struct Sink(Option<mpsc::Sender<TurnEvent>>);

impl Sink {
    async fn emit(&self, event: TurnEvent) {
        if let Some(tx) = &self.0 {
            // A closed receiver means the client went away. The turn still
            // finishes so that cleanup and auditing run; cancellation is the
            // mechanism for stopping early, not a dropped sink.
            let _ = tx.send(event).await;
        }
    }
}

pub struct Orchestrator {
    model: Option<Arc<dyn ModelProvider>>,
    executor: Arc<ToolExecutor>,
    /// Present when a durable store is configured. Without it the turn still
    /// stops at an approval, but the action is not persisted and cannot be
    /// resumed -- the honest degraded behaviour, reported to the client.
    approvals: Option<Arc<ApprovalCoordinator>>,
    context: Arc<dyn ContextProvider>,
    router: Arc<DeterministicRouter>,
    events: Option<EventBus>,
    config: OrchestratorConfig,
}

impl Orchestrator {
    pub fn builder() -> OrchestratorBuilder {
        OrchestratorBuilder::default()
    }

    pub fn config(&self) -> &OrchestratorConfig {
        &self.config
    }

    pub fn model_name(&self) -> Option<&str> {
        self.model.as_ref().map(|m| m.name())
    }

    /// The shared executor. Exposed so the approval resume path runs through
    /// exactly this instance rather than constructing a second one.
    pub fn executor(&self) -> &Arc<ToolExecutor> {
        &self.executor
    }

    /// Attaches the durable approval coordinator after construction.
    ///
    /// Exists to break a chicken-and-egg: the coordinator needs this
    /// orchestrator's executor, so it cannot be supplied to the builder.
    pub fn with_approvals(mut self, approvals: Option<Arc<ApprovalCoordinator>>) -> Self {
        self.approvals = approvals;
        self
    }

    /// Runs a turn to completion and returns the whole outcome.
    ///
    /// Convenience over [`Self::stream`] for callers that do not need
    /// incremental output, such as a future batch or background job path.
    pub async fn run(
        &self,
        request: TurnRequest,
        cancel: CancellationToken,
    ) -> Result<TurnOutcome, CoreError> {
        self.drive(request, cancel, &Sink(None)).await
    }

    /// Runs a turn, delivering [`TurnEvent`]s as they happen.
    ///
    /// The returned receiver closes when the turn ends. Failures arrive as a
    /// final [`TurnEvent::Failed`] rather than by dropping the channel, so a
    /// transport never has to guess why a stream stopped.
    pub fn stream(
        self: Arc<Self>,
        request: TurnRequest,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<TurnEvent> {
        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(async move {
            let turn_id = request.turn_id;
            let sink = Sink(Some(tx));

            match self.drive(request, cancel, &sink).await {
                Ok(outcome) => {
                    sink.emit(TurnEvent::Completed {
                        turn_id: outcome.turn_id,
                        message_id: outcome.message_id,
                        mode: outcome.mode,
                        rounds: outcome.rounds,
                    })
                    .await;
                }
                Err(error) => {
                    // `Display` is the user-safe text; the source chain, which
                    // may contain provider detail, is logged and not sent.
                    tracing::warn!(
                        turn_id = %turn_id,
                        code = error.code(),
                        error = ?error,
                        "turn ended without a final answer"
                    );
                    sink.emit(TurnEvent::Failed {
                        turn_id,
                        code: error.code(),
                        message: error.to_string(),
                    })
                    .await;
                }
            }
        });

        rx
    }

    /// Chooses how to answer, before any model is contacted.
    ///
    /// Returns the claiming handler alongside the plan so the turn does not have
    /// to route a second time -- routing twice would let a handler whose
    /// `matches` is not pure disagree with itself mid-turn.
    fn plan(
        &self,
        input: &NormalizedInput,
    ) -> (ExecutionPlan, Option<Arc<dyn DeterministicHandler>>) {
        if let Some(handler) = self.router.route(input) {
            let plan = ExecutionPlan {
                mode: ExecutionMode::Deterministic,
                handler: Some(handler.name().to_string()),
            };
            return (plan, Some(handler));
        }

        let plan = ExecutionPlan {
            mode: if self.executor.registry().is_empty() {
                ExecutionMode::Model
            } else {
                ExecutionMode::ModelWithTools
            },
            handler: None,
        };
        (plan, None)
    }

    #[tracing::instrument(
        name = "turn",
        skip_all,
        fields(
            turn_id = %request.turn_id,
            conversation_id = %request.conversation_id,
            user_id = %request.principal.user_id,
            mode = tracing::field::Empty,
            handler = tracing::field::Empty,
            rounds = tracing::field::Empty,
        )
    )]
    async fn drive(
        &self,
        request: TurnRequest,
        cancel: CancellationToken,
        sink: &Sink,
    ) -> Result<TurnOutcome, CoreError> {
        // The user's text is never recorded in a span field or a log line. Only
        // its length is, which is enough to correlate a slow turn with a large
        // input without putting message content into the log.
        let input = normalize(&request.input)?;
        tracing::debug!(input_chars = input.text.len(), "turn normalised");

        let context = self
            .context
            .assemble(&request)
            .await
            .map_err(CoreError::ContextError)?;

        let (plan, handler) = self.plan(&input);
        let span = tracing::Span::current();
        span.record("mode", tracing::field::debug(plan.mode));
        if let Some(handler) = &plan.handler {
            span.record("handler", handler.as_str());
        }

        self.publish(DomainEvent::TurnStarted {
            turn_id: request.turn_id,
            conversation_id: request.conversation_id,
        });

        sink.emit(TurnEvent::Started {
            turn_id: request.turn_id,
            mode: plan.mode,
        })
        .await;

        let result = match plan.mode {
            ExecutionMode::Deterministic => {
                let handler = handler.ok_or_else(|| {
                    CoreError::Internal("deterministic plan without a handler".into())
                })?;
                self.run_deterministic(&request, &input, &context, handler.as_ref(), sink)
                    .await
            }
            ExecutionMode::Model | ExecutionMode::ModelWithTools => {
                self.run_model(&request, &input, &context, plan.mode, cancel, sink)
                    .await
            }
        };

        match &result {
            Ok(outcome) => {
                span.record("rounds", outcome.rounds);
                self.publish(DomainEvent::TurnCompleted {
                    turn_id: request.turn_id,
                    mode: outcome.mode,
                    rounds: outcome.rounds,
                });
            }
            Err(error) => {
                self.publish(DomainEvent::TurnFailed {
                    turn_id: request.turn_id,
                    code: error.code().to_string(),
                });
            }
        }

        result
    }

    /// The fast path. No model is constructed, contacted or awaited.
    async fn run_deterministic(
        &self,
        request: &TurnRequest,
        input: &NormalizedInput,
        context: &TurnContext,
        handler: &dyn DeterministicHandler,
        sink: &Sink,
    ) -> Result<TurnOutcome, CoreError> {
        let message_id: MessageId = Uuid::new_v4();
        sink.emit(TurnEvent::AssistantStarted { message_id }).await;

        let text = handler.handle(input, request, context).await?;

        sink.emit(TurnEvent::AssistantDelta {
            message_id,
            text: text.clone(),
        })
        .await;

        Ok(TurnOutcome {
            turn_id: request.turn_id,
            message_id,
            mode: ExecutionMode::Deterministic,
            text,
            proposed: Vec::new(),
            executed: Vec::new(),
            rounds: 0,
        })
    }

    async fn run_model(
        &self,
        request: &TurnRequest,
        input: &NormalizedInput,
        context: &TurnContext,
        mode: ExecutionMode,
        cancel: CancellationToken,
        sink: &Sink,
    ) -> Result<TurnOutcome, CoreError> {
        // Fail before any work if this deployment has no provider at all.
        if self.model.is_none() {
            return Err(CoreError::NoModelProvider);
        }

        let mut messages = build_messages(&self.config, context, input);
        let tools = match mode {
            ExecutionMode::ModelWithTools => self.executor.registry().declarations(),
            _ => Vec::new(),
        };

        let message_id: MessageId = Uuid::new_v4();
        sink.emit(TurnEvent::AssistantStarted { message_id }).await;
        self.publish(DomainEvent::AssistantStarted {
            turn_id: request.turn_id,
        });

        // Assigned exactly once, on the pass where the model stops asking for tools.
        let answer: String;
        let mut proposed: Vec<ToolCall> = Vec::new();
        let mut executed: Vec<ToolResult> = Vec::new();
        let mut rounds = 0usize;

        loop {
            if cancel.is_cancelled() {
                return Err(CoreError::Cancelled);
            }

            let (text, calls) = self
                .one_model_pass(messages.clone(), tools.clone(), message_id, &cancel, sink)
                .await?;

            if calls.is_empty() {
                answer = text;
                break;
            }

            // The limit is checked before any tool runs, so exceeding it costs
            // nothing beyond the model call that proposed the calls.
            if rounds >= self.config.max_tool_rounds {
                tracing::warn!(
                    limit = self.config.max_tool_rounds,
                    "turn stopped at the tool round limit"
                );
                return Err(CoreError::IterationLimitExceeded {
                    limit: self.config.max_tool_rounds,
                });
            }

            proposed.extend(calls.iter().cloned());

            let results = self
                .run_tool_round(
                    &calls,
                    &request.principal,
                    request.turn_id,
                    request.conversation_id,
                    &cancel,
                    sink,
                )
                .await?;

            messages.push(Message::assistant_tool_calls(text, calls));
            for result in &results {
                messages.push(Message::tool_result(
                    result.call_id.clone(),
                    render_tool_result(result),
                ));
            }
            executed.extend(results);

            rounds += 1;
        }

        Ok(TurnOutcome {
            turn_id: request.turn_id,
            message_id,
            mode,
            text: answer,
            proposed,
            executed,
            rounds,
        })
    }

    /// One provider call, streamed. Returns the accumulated text and any tool
    /// calls the model proposed.
    async fn one_model_pass(
        &self,
        messages: Vec<Message>,
        tools: Vec<assistant_tools::ToolSpec>,
        message_id: MessageId,
        cancel: &CancellationToken,
        sink: &Sink,
    ) -> Result<(String, Vec<ToolCall>), CoreError> {
        let model = self.model.as_ref().ok_or(CoreError::NoModelProvider)?;

        let request = GenerateRequest {
            model: self.config.model.clone(),
            system_prompt: self.config.system_prompt.clone(),
            messages,
            tools,
            max_output_tokens: None,
            temperature: None,
        };

        let mut stream = model.stream(request).await.map_err(CoreError::ModelError)?;

        let mut text = String::new();
        let mut calls = Vec::new();

        loop {
            let next = tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(CoreError::Cancelled),
                next = stream.next() => next,
            };

            let Some(chunk) = next else { break };

            match chunk.map_err(CoreError::ModelError)? {
                StreamChunk::Text(part) => {
                    text.push_str(&part);
                    sink.emit(TurnEvent::AssistantDelta {
                        message_id,
                        text: part,
                    })
                    .await;
                }
                StreamChunk::ToolCall(call) => calls.push(call),
                StreamChunk::Done(usage) => {
                    tracing::debug!(
                        input_tokens = usage.input_tokens,
                        output_tokens = usage.output_tokens,
                        "model pass finished"
                    );
                    break;
                }
            }
        }

        Ok((text, calls))
    }

    /// Resolves, authorises and runs one batch of proposed tool calls.
    ///
    /// The whole batch is authorised before any of it runs. That ordering is
    /// deliberate: if one call in a batch needs approval, nothing in that batch
    /// should have already taken effect by the time the user is asked.
    #[allow(clippy::too_many_arguments)]
    async fn run_tool_round(
        &self,
        calls: &[ToolCall],
        principal: &Principal,
        turn_id: Uuid,
        conversation_id: Uuid,
        cancel: &CancellationToken,
        sink: &Sink,
    ) -> Result<Vec<ToolResult>, CoreError> {
        let mut specs = Vec::with_capacity(calls.len());

        for call in calls {
            // Unknown tool: there is no spec to authorise against, so the turn
            // stops here rather than guessing.
            let spec = self.executor.resolve(call)?;

            // Minimal structural validation. Full JSON Schema validation of
            // `input_schema` is a later, separate piece of work; what matters
            // now is that a call whose arguments are not even an object never
            // reaches a tool.
            if !call.arguments.is_object() {
                return Err(CoreError::ToolValidationError {
                    name: call.name.clone(),
                    reason: "arguments must be a JSON object".to_string(),
                });
            }

            sink.emit(TurnEvent::ToolProposed {
                call_id: call.id.clone(),
                name: call.name.clone(),
                risk: spec.risk,
            })
            .await;
            self.publish(DomainEvent::ToolProposed {
                name: spec.name.clone(),
                risk: spec.risk,
            });

            match self.executor.decide(&spec, principal) {
                PermissionDecision::Allow => {}
                PermissionDecision::RequireApproval { reason } => {
                    // Persist before telling anyone. If the write fails the turn
                    // fails: emitting an approval the user could answer, backed
                    // by nothing, would be worse than stopping.
                    let approval_id = match &self.approvals {
                        Some(coordinator) => Some(
                            coordinator
                                .propose(
                                    call,
                                    principal,
                                    turn_id,
                                    conversation_id,
                                    spec.risk,
                                    &reason,
                                )
                                .await?
                                .id,
                        ),
                        None => {
                            tracing::warn!(
                                tool = %spec.name,
                                "no durable store configured; this approval cannot be resumed"
                            );
                            None
                        }
                    };

                    sink.emit(TurnEvent::ApprovalRequired {
                        call_id: call.id.clone(),
                        name: spec.name.clone(),
                        risk: spec.risk,
                        reason: reason.clone(),
                        approval_id,
                        summary: summarize(&spec.name, &call.arguments),
                    })
                    .await;
                    self.publish(DomainEvent::ApprovalRequested {
                        name: spec.name.clone(),
                        risk: spec.risk,
                    });

                    return Err(CoreError::ApprovalRequired {
                        name: spec.name.clone(),
                        risk: spec.risk,
                        reason,
                    });
                }
                PermissionDecision::Deny { reason } => {
                    return Err(CoreError::PermissionDenied {
                        name: spec.name.clone(),
                        reason,
                    });
                }
            }

            specs.push(spec);
        }

        // Concurrency rule, deliberately conservative: run in parallel only when
        // every call in the batch is read-only (`Green`) and no tool appears
        // twice. Anything else -- a write, a repeat of the same tool, an unclear
        // ordering -- runs sequentially. Correctness before latency.
        let all_read_only = specs.iter().all(|spec| spec.risk == RiskLevel::Green);
        let names: std::collections::HashSet<&str> =
            specs.iter().map(|spec| spec.name.as_str()).collect();
        let all_distinct = names.len() == specs.len();

        for call in calls {
            sink.emit(TurnEvent::ToolStarted {
                call_id: call.id.clone(),
                name: call.name.clone(),
            })
            .await;
        }

        let results = if all_read_only && all_distinct && calls.len() > 1 {
            tracing::debug!(
                count = calls.len(),
                "running read-only tool calls concurrently"
            );
            let futures = calls
                .iter()
                .map(|call| self.executor.execute(call, principal, cancel));
            futures::future::try_join_all(futures).await?
        } else {
            let mut results = Vec::with_capacity(calls.len());
            for call in calls {
                results.push(self.executor.execute(call, principal, cancel).await?);
            }
            results
        };

        for result in &results {
            sink.emit(TurnEvent::ToolCompleted {
                call_id: result.call_id.clone(),
                name: result.name.clone(),
                ok: result.result.is_ok(),
            })
            .await;
            self.publish(DomainEvent::ToolCompleted {
                name: result.name.clone(),
                ok: result.result.is_ok(),
            });
        }

        Ok(results)
    }

    fn publish(&self, event: DomainEvent) {
        if let Some(bus) = &self.events {
            bus.publish(event);
        }
    }
}

impl std::fmt::Debug for Orchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Orchestrator")
            .field("model", &self.model_name())
            .field("executor", &self.executor)
            .field("router", &self.router)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Turns context and input into the message list sent to the provider.
///
/// History is replayed oldest-first and the current user turn goes last, which
/// is the ordering every provider expects.
fn build_messages(
    config: &OrchestratorConfig,
    context: &TurnContext,
    input: &NormalizedInput,
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(context.history.len() + 2);

    if let Some(prompt) = &config.system_prompt {
        messages.push(Message::system(prompt.clone()));
    }

    if !context.facts.is_empty() {
        messages.push(Message::system(format!(
            "Relevant context:\n{}",
            context.facts.join("\n")
        )));
    }

    for entry in &context.history {
        messages.push(match entry.role {
            ContextRole::User => Message::user(entry.content.clone()),
            ContextRole::Assistant => Message::assistant(entry.content.clone()),
        });
    }

    messages.push(Message::user(input.text.clone()));
    messages
}

/// Renders a tool result as the text handed back to the model.
fn render_tool_result(result: &ToolResult) -> String {
    match &result.result {
        Ok(value) => value.to_string(),
        Err(message) => format!(
            "{{\"error\":{}}}",
            serde_json::Value::String(message.clone())
        ),
    }
}

/// Builds an [`Orchestrator`] with every dependency supplied explicitly.
///
/// Defaults are the inert ones: no model, no tools, no context, no events. A
/// caller has to opt into each capability, so nothing is silently enabled.
#[derive(Default)]
pub struct OrchestratorBuilder {
    model: Option<Arc<dyn ModelProvider>>,
    registry: Option<Arc<ToolRegistry>>,
    policy: Option<Arc<dyn PermissionPolicy>>,
    context: Option<Arc<dyn ContextProvider>>,
    router: Option<Arc<DeterministicRouter>>,
    events: Option<EventBus>,
    approvals: Option<Arc<ApprovalCoordinator>>,
    config: Option<OrchestratorConfig>,
}

impl OrchestratorBuilder {
    pub fn model(mut self, model: Arc<dyn ModelProvider>) -> Self {
        self.model = Some(model);
        self
    }

    /// Sets the provider when there is one.
    ///
    /// A deployment with no provider configured is a supported state: the
    /// deterministic fast path still works, and a turn that needs a model fails
    /// with [`CoreError::NoModelProvider`] rather than fabricating an answer.
    pub fn maybe_model(mut self, model: Option<Arc<dyn ModelProvider>>) -> Self {
        self.model = model;
        self
    }

    pub fn registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn policy(mut self, policy: Arc<dyn PermissionPolicy>) -> Self {
        self.policy = Some(policy);
        self
    }

    pub fn context(mut self, context: Arc<dyn ContextProvider>) -> Self {
        self.context = Some(context);
        self
    }

    pub fn router(mut self, router: Arc<DeterministicRouter>) -> Self {
        self.router = Some(router);
        self
    }

    pub fn events(mut self, events: EventBus) -> Self {
        self.events = Some(events);
        self
    }

    /// Supplies the durable approval coordinator. Absent means approvals stop
    /// the turn but are not persisted.
    pub fn approvals(mut self, approvals: Arc<ApprovalCoordinator>) -> Self {
        self.approvals = Some(approvals);
        self
    }

    pub fn maybe_approvals(mut self, approvals: Option<Arc<ApprovalCoordinator>>) -> Self {
        self.approvals = approvals;
        self
    }

    pub fn config(mut self, config: OrchestratorConfig) -> Self {
        self.config = Some(config);
        self
    }

    pub fn build(self) -> Orchestrator {
        let registry = self
            .registry
            .unwrap_or_else(|| Arc::new(ToolRegistry::new()));
        let policy = self
            .policy
            .unwrap_or_else(|| Arc::new(RiskBasedPolicy::new()));

        Orchestrator {
            model: self.model,
            executor: Arc::new(ToolExecutor::new(registry, policy)),
            approvals: self.approvals,
            context: self
                .context
                .unwrap_or_else(|| Arc::new(EmptyContextProvider)),
            router: self
                .router
                .unwrap_or_else(|| Arc::new(DeterministicRouter::new())),
            events: self.events,
            config: self.config.unwrap_or_default(),
        }
    }
}
