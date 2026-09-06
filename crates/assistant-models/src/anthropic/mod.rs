//! Anthropic Messages API provider.
//!
//! The first production-capable [`ModelProvider`]. It is a deliberate, small
//! HTTP client rather than an SDK: the surface this application needs is one
//! endpoint, one streaming format and one error shape, and `reqwest` is already
//! the project's HTTP dependency. See docs/DECISIONS.md ADR-0018.
//!
//! What lives here and nowhere else:
//!
//! * the endpoint, the `x-api-key` and `anthropic-version` headers,
//! * the request and response wire format ([`wire`]),
//! * the SSE stream format ([`sse`]),
//! * the mapping from an HTTP status onto a structured [`ModelError`].
//!
//! What deliberately does not live here: the API key's *origin* (the server's
//! configuration layer reads the environment), the tool definitions (generated
//! from the registry's authoritative `ToolSpec`s), the risk of a tool, and any
//! decision about whether a call may run.

pub mod config;
mod sse;
mod wire;

use std::{collections::VecDeque, pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::{
    Capability, ChunkStream, GenerateRequest, GenerateResponse, ModelError, ModelProvider, Role,
    StreamChunk,
};

pub use config::{AnthropicConfig, Effort};

/// The API version this build was written against. Sent on every request so a
/// server-side change cannot silently alter the response shape.
const API_VERSION: &str = "2023-06-01";

pub struct AnthropicModelProvider {
    http: reqwest::Client,
    config: AnthropicConfig,
    capabilities: Vec<Capability>,
}

impl AnthropicModelProvider {
    /// Builds a provider with its own connection pool.
    ///
    /// One client for the process: TLS handshakes and connection setup are a
    /// meaningful part of first-token latency, and a per-request client throws
    /// that away every turn.
    pub fn new(config: AnthropicConfig) -> Result<Self, ModelError> {
        if config.api_key().trim().is_empty() {
            return Err(ModelError::AuthFailed);
        }

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Applies between reads, not to the whole response: a stream that
            // is still producing tokens is not late.
            .read_timeout(config.timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .map_err(|error| ModelError::Transport(error.to_string()))?;

        Ok(Self {
            http,
            config,
            capabilities: vec![
                Capability::Generate,
                Capability::Stream,
                Capability::ToolUse,
            ],
        })
    }

    pub fn config(&self) -> &AnthropicConfig {
        &self.config
    }

    /// Sends one request, retrying only where a retry is provably safe.
    ///
    /// The rule is narrow on purpose. A retry costs money and, in a turn that
    /// has already run tools, could ask the model to redo consequential work,
    /// so a request carrying tool results is never retried. What is retried is
    /// a transport failure on a request with no tool results: nothing happened,
    /// and nothing was charged. See ADR-0019.
    async fn send(&self, body: &wire::WireRequest, retryable: bool) -> Result<Value, ModelError> {
        let attempts = self.attempts(retryable);
        let mut last = ModelError::Transport("no attempt was made".into());

        for attempt in 0..attempts {
            if attempt > 0 {
                // Fixed, short backoff. Anything cleverer needs evidence this
                // one is insufficient.
                tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                tracing::warn!(attempt, "retrying provider request after transport failure");
            }

            match self.request(body).await {
                Ok(response) => return Ok(response),
                Err(error) if error.is_transient() => last = error,
                Err(error) => return Err(error),
            }
        }

        Err(last)
    }

    fn attempts(&self, retryable: bool) -> u32 {
        if retryable {
            self.config.max_transport_retries.saturating_add(1)
        } else {
            1
        }
    }

    /// One non-streaming HTTP round trip.
    async fn request(&self, body: &wire::WireRequest) -> Result<Value, ModelError> {
        let response = self
            .post(body)
            .timeout(self.config.timeout)
            .send()
            .await
            .map_err(transport_error)?;

        let status = response.status();
        let retry_after = retry_after(response.headers());
        let text = response.text().await.map_err(transport_error)?;

        if !status.is_success() {
            return Err(status_error(status, &text, retry_after));
        }

        serde_json::from_str(&text).map_err(|error| {
            ModelError::MalformedResponse(format!("response was not valid JSON: {error}"))
        })
    }

    fn post(&self, body: &wire::WireRequest) -> reqwest::RequestBuilder {
        self.http
            .post(self.config.messages_url())
            // `x-api-key`, never an `Authorization` header, and never logged:
            // no tracing span in this crate records a header.
            .header("x-api-key", self.config.api_key())
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(body)
    }
}

#[async_trait]
impl ModelProvider for AnthropicModelProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse, ModelError> {
        let body = wire::build_request(&self.config, &request, false);
        let response = self.send(&body, is_retryable(&request)).await?;
        let parsed = wire::parse_message(&response)?;

        if parsed.stop_reason.as_deref() == Some("refusal") {
            return Err(ModelError::Refused {
                category: parsed.stop_category,
            });
        }

        Ok(GenerateResponse {
            text: parsed.text,
            tool_calls: parsed.tool_calls,
            usage: parsed.usage,
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<ChunkStream, ModelError> {
        if !self.config.streaming {
            // Honest degradation: the caller still gets the chunk contract, it
            // simply arrives all at once. Nothing pretends to be incremental.
            let response = self.generate(request).await?;
            let mut items: Vec<Result<StreamChunk, ModelError>> = Vec::new();
            if let Some(text) = response.text {
                items.push(Ok(StreamChunk::Text(text)));
            }
            items.extend(
                response
                    .tool_calls
                    .into_iter()
                    .map(|call| Ok(StreamChunk::ToolCall(call))),
            );
            items.push(Ok(StreamChunk::Done(response.usage)));
            return Ok(Box::pin(futures::stream::iter(items)));
        }

        let body = wire::build_request(&self.config, &request, true);
        let attempts = self.attempts(is_retryable(&request));
        let mut last = ModelError::Transport("no attempt was made".into());

        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                tracing::warn!(attempt, "retrying provider stream after transport failure");
            }

            // Only the *opening* of the stream is retried. Once bytes have been
            // delivered to the caller, a failure is reported, never replayed:
            // replaying would duplicate text the user has already seen.
            match self.post(&body).send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(into_chunk_stream(response.bytes_stream()));
                    }

                    let retry_after = retry_after(response.headers());
                    let text = response.text().await.unwrap_or_default();
                    let error = status_error(status, &text, retry_after);
                    if !error.is_transient() {
                        return Err(error);
                    }
                    last = error;
                }
                Err(error) => {
                    let error = transport_error(error);
                    if !error.is_transient() {
                        return Err(error);
                    }
                    last = error;
                }
            }
        }

        Err(last)
    }
}

impl std::fmt::Debug for AnthropicModelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicModelProvider")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Whether resending this exact request could repeat consequential work.
///
/// A request carrying tool results belongs to a turn that has already executed
/// something. Even though the retry would only re-ask the model, the model's
/// next move could be to ask for the same write again, so the request is left
/// alone and the turn fails cleanly.
fn is_retryable(request: &GenerateRequest) -> bool {
    !request
        .messages
        .iter()
        .any(|message| message.role == Role::Tool)
}

/// Wraps the HTTP body stream in the provider-neutral chunk stream.
///
/// Dropping the returned stream drops the body, which closes the connection and
/// stops the provider generating. That is how a cancelled turn stops costing
/// money: the orchestrator drops the stream, and nothing else has to be told.
fn into_chunk_stream<S, B>(body: S) -> ChunkStream
where
    S: Stream<Item = Result<B, reqwest::Error>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
{
    struct State<S> {
        body: Pin<Box<S>>,
        decoder: sse::SseDecoder,
        pending: VecDeque<Result<StreamChunk, ModelError>>,
        finished: bool,
    }

    let state = State {
        body: Box::pin(body),
        decoder: sse::SseDecoder::new(),
        pending: VecDeque::new(),
        finished: false,
    };

    Box::pin(futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                // An error terminates the stream: continuing to read after one
                // would deliver chunks the consumer has already given up on.
                if item.is_err() {
                    state.finished = true;
                    state.pending.clear();
                }
                return Some((item, state));
            }

            if state.finished {
                return None;
            }

            match state.body.next().await {
                Some(Ok(bytes)) => state.decoder.push(bytes.as_ref(), &mut state.pending),
                Some(Err(error)) => state.pending.push_back(Err(transport_error(error))),
                None => {
                    state.decoder.finish(&mut state.pending);
                    state.finished = true;
                }
            }
        }
    }))
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Maps an HTTP status onto the structured error type.
///
/// The body is read for its `error.message` only, and only into the variant's
/// log-side detail. Request headers are never touched here, so no code path
/// exists that could put the API key into an error.
fn status_error(
    status: reqwest::StatusCode,
    body: &str,
    retry_after: Option<Duration>,
) -> ModelError {
    let detail = wire::error_detail(body);

    match status.as_u16() {
        401 | 403 => ModelError::AuthFailed,
        408 => ModelError::Timeout,
        429 => ModelError::RateLimited { retry_after },
        // 529 is the provider's "overloaded"; it is retryable, a 4xx is not.
        529 => ModelError::Unavailable(detail),
        code if (500..600).contains(&code) => ModelError::Unavailable(detail),
        code if (400..500).contains(&code) => ModelError::InvalidRequest(detail),
        code => ModelError::Rejected(format!("unexpected status {code}: {detail}")),
    }
}

fn transport_error(error: reqwest::Error) -> ModelError {
    if error.is_timeout() {
        ModelError::Timeout
    } else if error.is_connect() || error.is_request() {
        ModelError::Unavailable(error.to_string())
    } else {
        ModelError::Transport(error.to_string())
    }
}
