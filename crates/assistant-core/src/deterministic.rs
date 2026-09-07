//! The deterministic fast path.
//!
//! Some questions have exact answers that code already knows. Routing those
//! through a language model costs money, adds a network round trip to a
//! latency-critical product, and can return something wrong. So routing happens
//! *before* the model, in code, and the model is never asked whether it should
//! have been consulted -- by the time it could answer that, the cost is paid.
//!
//! A handler must be genuinely deterministic: same input and same system state
//! produce the same answer, with no inference anywhere in the path.

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    CoreError,
    registry::ToolRegistry,
    turn::{NormalizedInput, TurnContext, TurnRequest},
};

#[async_trait]
pub trait DeterministicHandler: Send + Sync {
    /// Stable name, recorded in the execution plan so logs can explain why a
    /// turn skipped the model.
    fn name(&self) -> &str;

    /// Whether this handler can answer the input exactly.
    ///
    /// Must be cheap and side-effect free: it is called for every turn, and a
    /// handler that is unsure should return `false` and let the model take it.
    fn matches(&self, input: &NormalizedInput) -> bool;

    async fn handle(
        &self,
        input: &NormalizedInput,
        request: &TurnRequest,
        context: &TurnContext,
    ) -> Result<String, CoreError>;
}

/// Picks a handler, or none.
///
/// First match wins, so registration order is precedence. Kept as an explicit
/// type rather than a bare `Vec` so that a future router can weigh handlers
/// without changing the orchestrator.
#[derive(Default)]
pub struct DeterministicRouter {
    handlers: Vec<Arc<dyn DeterministicHandler>>,
}

impl DeterministicRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, handler: Arc<dyn DeterministicHandler>) {
        self.handlers.push(handler);
    }

    pub fn with(mut self, handler: Arc<dyn DeterministicHandler>) -> Self {
        self.register(handler);
        self
    }

    pub fn route(&self, input: &NormalizedInput) -> Option<Arc<dyn DeterministicHandler>> {
        self.handlers
            .iter()
            .find(|handler| handler.matches(input))
            .cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }
}

impl std::fmt::Debug for DeterministicRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.handlers.iter().map(|h| h.name()).collect();
        f.debug_struct("DeterministicRouter")
            .field("handlers", &names)
            .finish()
    }
}

/// Answers "what is your status?" from facts the process already holds.
///
/// This is a real deterministic operation, not a demonstration: the answer is
/// assembled from the injected model provider's name and the actual contents of
/// the tool registry. Nothing is invented, and no fake task or calendar store
/// was created to give the fast path something to do.
pub struct AssistantStatusHandler {
    /// Name of the configured provider, or `None` when none is injected.
    model_name: Option<String>,
    registry: Arc<ToolRegistry>,
}

impl AssistantStatusHandler {
    pub fn new(model_name: Option<String>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            model_name,
            registry,
        }
    }

    /// Phrases that mean "report your own state".
    ///
    /// Matching is exact-substring on normalised, lowercased text. Deliberately
    /// narrow: a handler that guesses would hijack turns the model should have
    /// answered, and a missed match only costs a model call.
    const TRIGGERS: &'static [&'static str] = &[
        "status",
        "health",
        "are you online",
        "are you working",
        "are you there",
    ];
}

#[async_trait]
impl DeterministicHandler for AssistantStatusHandler {
    fn name(&self) -> &str {
        "assistant.status"
    }

    fn matches(&self, input: &NormalizedInput) -> bool {
        Self::TRIGGERS
            .iter()
            .any(|trigger| input.matchable.contains(trigger))
    }

    async fn handle(
        &self,
        _input: &NormalizedInput,
        _request: &TurnRequest,
        _context: &TurnContext,
    ) -> Result<String, CoreError> {
        let model = match &self.model_name {
            Some(name) => format!("Model provider: {name}."),
            None => "No model provider is configured.".to_string(),
        };

        let tools = match self.registry.len() {
            0 => "No tools are registered.".to_string(),
            1 => "1 tool is registered.".to_string(),
            n => format!("{n} tools are registered."),
        };

        Ok(format!("Assistant core is running. {model} {tools}"))
    }
}

/// Answers `remember that ...` from code, without going near a model.
///
/// This is the explicit-memory path: a deterministic parser detects the
/// instruction, the application constructs a [`assistant_memory::NewMemory`]
/// with `explicit_user_input` provenance, and the store persists it. Secret-
/// looking content is refused, not stored. The user sees a plain confirmation
/// with no fabricated content.
///
/// A model is never asked whether to accept, and there is no path that lets
/// model output arrive here as an "explicit" instruction: this handler reads
/// the raw request text.
pub struct RememberMemoryHandler {
    store: Arc<dyn assistant_memory::MemoryStore>,
}

impl RememberMemoryHandler {
    pub fn new(store: Arc<dyn assistant_memory::MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl DeterministicHandler for RememberMemoryHandler {
    fn name(&self) -> &str {
        "memory.remember"
    }

    fn matches(&self, input: &NormalizedInput) -> bool {
        assistant_memory::parse_explicit_memory(&input.text).is_some()
    }

    async fn handle(
        &self,
        _input: &NormalizedInput,
        request: &TurnRequest,
        _context: &TurnContext,
    ) -> Result<String, CoreError> {
        let Some(instruction) = assistant_memory::parse_explicit_memory(&request.input) else {
            // `matches` said yes and `handle` said no: fall through to the
            // model rather than fabricate a confirmation for something we did
            // not store.
            return Err(CoreError::InvalidInput);
        };

        let new_memory = assistant_memory::NewMemory::explicit(
            request.principal.user_id,
            instruction.kind,
            instruction.content.clone(),
        );

        match self.store.create(new_memory).await {
            Ok(stored) => Ok(format!(
                "Saved as {kind}: \"{content}\".",
                kind = stored.kind.as_str(),
                content = instruction.content
            )),
            Err(assistant_memory::MemoryError::SecretLike) => {
                Ok("That looks like a credential, so I did not save it. \
                 If you meant to store a preference or fact, rephrase it without the secret."
                    .to_string())
            }
            Err(assistant_memory::MemoryError::Invalid(reason)) => {
                Ok(format!("I could not save that memory: {reason}."))
            }
            Err(assistant_memory::MemoryError::NotFound(_)) => {
                // A create call cannot produce NotFound. Treat as an internal
                // fault rather than a user-safe outcome.
                Err(CoreError::InvalidInput)
            }
            Err(assistant_memory::MemoryError::Backend(reason)) => {
                tracing::warn!(%reason, "memory backend failure on explicit remember");
                Ok("I could not save that memory just now. Please try again in a moment.".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{testing::*, turn::normalize};
    use uuid::Uuid;

    fn handler(model: Option<&str>, tools: usize) -> AssistantStatusHandler {
        let mut registry = ToolRegistry::new();
        for i in 0..tools {
            registry.register(Arc::new(EchoTool::green(&format!("t{i}.read"))));
        }
        AssistantStatusHandler::new(model.map(String::from), Arc::new(registry))
    }

    #[test]
    fn status_questions_match_regardless_of_casing_or_spacing() {
        let handler = handler(None, 0);
        for phrasing in [
            "status",
            "What is your STATUS?",
            "  are   you   online ?  ",
            "give me a health check",
        ] {
            let input = normalize(phrasing).expect("accepted");
            assert!(handler.matches(&input), "should match: {phrasing}");
        }
    }

    #[test]
    fn unrelated_questions_are_left_to_the_model() {
        let handler = handler(None, 0);
        for phrasing in [
            "write me a study plan for compiler design",
            "when am I free on Thursday",
            "read this pdf",
        ] {
            let input = normalize(phrasing).expect("accepted");
            assert!(!handler.matches(&input), "should not match: {phrasing}");
        }
    }

    #[tokio::test]
    async fn the_answer_reports_real_state_not_a_canned_string() {
        let with_nothing = handler(None, 0);
        let request = TurnRequest::new(Uuid::new_v4(), dev_principal(), "status");
        let context = TurnContext::default();
        let input = normalize("status").expect("accepted");

        let answer = with_nothing
            .handle(&input, &request, &context)
            .await
            .expect("answered");
        assert!(answer.contains("No model provider is configured"));
        assert!(answer.contains("No tools are registered"));

        let with_things = handler(Some("mock"), 3);
        let answer = with_things
            .handle(&input, &request, &context)
            .await
            .expect("answered");
        assert!(answer.contains("mock"), "unexpected: {answer}");
        assert!(
            answer.contains("3 tools are registered"),
            "unexpected: {answer}"
        );
    }

    #[tokio::test]
    async fn remember_handler_persists_the_stated_content_without_a_model() {
        let store: Arc<dyn assistant_memory::MemoryStore> =
            Arc::new(assistant_memory::InMemoryMemoryStore::new());
        let handler = RememberMemoryHandler::new(store.clone());
        let input = normalize("Remember that I prefer concise answers.").expect("accepted");
        let request = TurnRequest::new(Uuid::new_v4(), dev_principal(), &input.text);
        assert!(handler.matches(&input));

        let answer = handler
            .handle(&input, &request, &TurnContext::default())
            .await
            .expect("answered");
        assert!(answer.contains("preference"), "unexpected: {answer}");
        assert!(
            answer.contains("I prefer concise answers"),
            "unexpected: {answer}"
        );

        let hits = store
            .search(assistant_memory::MemoryQuery::active(
                request.principal.user_id,
            ))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, assistant_memory::MemoryKind::Preference);
        assert_eq!(
            hits[0].provenance.source_kind,
            assistant_memory::MemorySource::ExplicitUserInput
        );
    }

    #[tokio::test]
    async fn remember_handler_refuses_secret_like_content_and_does_not_store_it() {
        let store: Arc<dyn assistant_memory::MemoryStore> =
            Arc::new(assistant_memory::InMemoryMemoryStore::new());
        let handler = RememberMemoryHandler::new(store.clone());
        let text = "Remember that my api_key=abcdef1234567890abcdef1234567890";
        let input = normalize(text).expect("accepted");
        let request = TurnRequest::new(Uuid::new_v4(), dev_principal(), &input.text);
        let answer = handler
            .handle(&input, &request, &TurnContext::default())
            .await
            .expect("answered");
        assert!(
            answer.to_lowercase().contains("credential"),
            "unexpected: {answer}"
        );

        let hits = store
            .search(assistant_memory::MemoryQuery::active(
                request.principal.user_id,
            ))
            .await
            .unwrap();
        assert!(hits.is_empty(), "secret content was persisted");
    }

    #[test]
    fn the_router_returns_the_first_matching_handler() {
        let router = DeterministicRouter::new().with(Arc::new(handler(None, 0)));

        let matched = router.route(&normalize("status").expect("accepted"));
        assert_eq!(
            matched.map(|h| h.name().to_string()),
            Some("assistant.status".into())
        );

        assert!(
            router
                .route(&normalize("plan my week").expect("accepted"))
                .is_none()
        );
    }
}
