//! A deterministic in-process [`ModelProvider`] for tests.
//!
//! Behind the `mock` feature, so it is never linked into a release binary. It
//! makes no network calls and contains no randomness: the same script always
//! produces the same sequence of responses, which is what makes assertions about
//! tool loops and streaming meaningful.
//!
//! It also records every request it received and counts invocations, so a test
//! can prove the deterministic fast path never reached a model -- an assertion
//! that cannot be made from a return value alone.

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use assistant_tools::ToolCall;
use async_trait::async_trait;
use futures::stream;

use crate::{
    Capability, ChunkStream, GenerateRequest, GenerateResponse, ModelError, ModelProvider,
    StreamChunk, Usage,
};

/// What the mock returns for one invocation.
#[derive(Debug, Clone)]
pub enum MockResponse {
    /// An ordinary assistant answer.
    Text(String),
    /// One or more tool calls and no prose.
    ToolCalls(Vec<ToolCall>),
    /// Prose plus tool calls, as a real provider often returns.
    TextWithToolCalls { text: String, calls: Vec<ToolCall> },
    /// The provider failed.
    Error(String),
}

impl MockResponse {
    pub fn text(value: &str) -> Self {
        Self::Text(value.to_string())
    }

    /// A single tool call with the given name and arguments.
    pub fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> Self {
        Self::ToolCalls(vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
        }])
    }

    /// A call whose arguments are not a JSON object, which the core must reject
    /// before the tool is ever reached.
    pub fn malformed_tool_call(id: &str, name: &str) -> Self {
        Self::ToolCalls(vec![ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: serde_json::json!("this is not an argument object"),
        }])
    }
}

/// Scripted provider. Responses are consumed in order.
///
/// When the script is exhausted the provider repeats [`Self::fallback`], which
/// defaults to a plain text answer. That keeps a test that under-specifies its
/// script from hanging or panicking in a confusing place.
pub struct MockModelProvider {
    name: String,
    capabilities: Vec<Capability>,
    script: Mutex<std::collections::VecDeque<MockResponse>>,
    fallback: MockResponse,
    calls: AtomicUsize,
    requests: Mutex<Vec<GenerateRequest>>,
}

impl MockModelProvider {
    pub fn new(script: Vec<MockResponse>) -> Self {
        Self {
            name: "mock".to_string(),
            capabilities: vec![
                Capability::Generate,
                Capability::Stream,
                Capability::ToolUse,
            ],
            script: Mutex::new(script.into()),
            fallback: MockResponse::Text("done".to_string()),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// A provider that always answers with the same text.
    pub fn always(text: &str) -> Self {
        Self::new(vec![]).with_fallback(MockResponse::text(text))
    }

    pub fn with_name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    pub fn with_fallback(mut self, response: MockResponse) -> Self {
        self.fallback = response;
        self
    }

    /// Restricts what the provider claims it can do, so callers can test
    /// capability negotiation.
    pub fn with_capabilities(mut self, capabilities: Vec<Capability>) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// How many times `generate` or `stream` was entered.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Every request the provider received, in order.
    pub fn requests(&self) -> Vec<GenerateRequest> {
        self.requests.lock().expect("not poisoned").clone()
    }

    fn next(&self, request: GenerateRequest) -> MockResponse {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().expect("not poisoned").push(request);
        self.script
            .lock()
            .expect("not poisoned")
            .pop_front()
            .unwrap_or_else(|| self.fallback.clone())
    }
}

/// Splits text into deterministic chunks so streaming is observably incremental.
///
/// Word-boundary chunks with the separating space retained, so concatenating
/// every chunk reproduces the original string exactly.
fn chunks_of(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = text
        .split_inclusive(' ')
        .map(|part| part.to_string())
        .collect();
    if chunks.is_empty() && !text.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}

#[async_trait]
impl ModelProvider for MockModelProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse, ModelError> {
        match self.next(request) {
            MockResponse::Text(text) => Ok(GenerateResponse {
                text: Some(text),
                tool_calls: Vec::new(),
                usage: Usage::default(),
            }),
            MockResponse::ToolCalls(calls) => Ok(GenerateResponse {
                text: None,
                tool_calls: calls,
                usage: Usage::default(),
            }),
            MockResponse::TextWithToolCalls { text, calls } => Ok(GenerateResponse {
                text: Some(text),
                tool_calls: calls,
                usage: Usage::default(),
            }),
            MockResponse::Error(message) => Err(ModelError::Rejected(message)),
        }
    }

    async fn stream(&self, request: GenerateRequest) -> Result<ChunkStream, ModelError> {
        let response = self.next(request);

        // An error surfaces when the stream is opened, matching how a provider
        // rejects a request before emitting anything.
        let (text, calls) = match response {
            MockResponse::Text(text) => (Some(text), Vec::new()),
            MockResponse::ToolCalls(calls) => (None, calls),
            MockResponse::TextWithToolCalls { text, calls } => (Some(text), calls),
            MockResponse::Error(message) => return Err(ModelError::Rejected(message)),
        };

        let mut items: Vec<Result<StreamChunk, ModelError>> = Vec::new();
        if let Some(text) = text {
            items.extend(
                chunks_of(&text)
                    .into_iter()
                    .map(|chunk| Ok(StreamChunk::Text(chunk))),
            );
        }
        items.extend(
            calls
                .into_iter()
                .map(|call| Ok(StreamChunk::ToolCall(call))),
        );
        items.push(Ok(StreamChunk::Done(Usage::default())));

        Ok(Box::pin(stream::iter(items)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn request() -> GenerateRequest {
        GenerateRequest {
            model: crate::ModelId("mock".into()),
            system_prompt: None,
            messages: Vec::new(),
            tools: Vec::new(),
            max_output_tokens: None,
            temperature: None,
        }
    }

    #[tokio::test]
    async fn the_script_is_consumed_in_order_then_falls_back() {
        let provider = MockModelProvider::new(vec![
            MockResponse::text("first"),
            MockResponse::text("second"),
        ])
        .with_fallback(MockResponse::text("fallback"));

        for expected in ["first", "second", "fallback", "fallback"] {
            let response = provider.generate(request()).await.expect("ok");
            assert_eq!(response.text.as_deref(), Some(expected));
        }
        assert_eq!(provider.calls(), 4);
    }

    #[tokio::test]
    async fn streaming_is_incremental_and_reassembles_exactly() {
        let provider = MockModelProvider::always("the quick brown fox");
        let mut stream = provider.stream(request()).await.expect("opened");

        let mut text = String::new();
        let mut chunk_count = 0;
        let mut saw_done = false;

        while let Some(chunk) = stream.next().await {
            match chunk.expect("no error") {
                StreamChunk::Text(part) => {
                    chunk_count += 1;
                    text.push_str(&part);
                }
                StreamChunk::Done(_) => saw_done = true,
                StreamChunk::ToolCall(_) => panic!("no tool call expected"),
            }
        }

        assert_eq!(text, "the quick brown fox");
        assert!(chunk_count > 1, "stream was not incremental");
        assert!(saw_done);
    }

    #[tokio::test]
    async fn an_error_response_surfaces_when_the_stream_is_opened() {
        let provider = MockModelProvider::new(vec![MockResponse::Error("overloaded".into())]);
        assert!(provider.stream(request()).await.is_err());
    }

    #[tokio::test]
    async fn multiple_tool_calls_are_returned_together() {
        let provider = MockModelProvider::new(vec![MockResponse::ToolCalls(vec![
            ToolCall {
                id: "a".into(),
                name: "notes.read".into(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                id: "b".into(),
                name: "tasks.read".into(),
                arguments: serde_json::json!({}),
            },
        ])]);

        let response = provider.generate(request()).await.expect("ok");
        assert_eq!(response.tool_calls.len(), 2);
    }

    #[tokio::test]
    async fn requests_are_recorded_for_inspection() {
        let provider = MockModelProvider::always("hi");
        provider.generate(request()).await.expect("ok");
        assert_eq!(provider.requests().len(), 1);
    }
}
