//! OpenAI Chat Completions API provider.

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

pub use config::OpenAIConfig;

pub struct OpenAIModelProvider {
    http: reqwest::Client,
    config: OpenAIConfig,
    capabilities: Vec<Capability>,
}

impl OpenAIModelProvider {
    pub fn new(config: OpenAIConfig) -> Result<Self, ModelError> {
        if config.api_key().trim().is_empty() {
            return Err(ModelError::AuthFailed);
        }

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
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

    pub fn config(&self) -> &OpenAIConfig {
        &self.config
    }

    async fn send(&self, body: &wire::WireRequest, retryable: bool) -> Result<Value, ModelError> {
        let attempts = self.attempts(retryable);
        let mut last = ModelError::Transport("no attempt was made".into());

        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
                tracing::warn!(attempt, "retrying OpenAI request after transport failure");
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
            .post(self.config.chat_completions_url())
            .bearer_auth(self.config.api_key())
            .header("content-type", "application/json")
            .json(body)
    }
}

#[async_trait]
impl ModelProvider for OpenAIModelProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse, ModelError> {
        let body = wire::build_request(&self.config, &request, false);
        let response = self.send(&body, is_retryable(&request)).await?;
        let (text, tool_calls, usage) = wire::parse_response(&response)?;

        Ok(GenerateResponse {
            text,
            tool_calls,
            usage,
        })
    }

    async fn stream(&self, request: GenerateRequest) -> Result<ChunkStream, ModelError> {
        let body = wire::build_request(&self.config, &request, true);

        let response = self.post(&body).send().await.map_err(transport_error)?;

        let status = response.status();
        if !status.is_success() {
            let retry_after = retry_after(response.headers());
            let text = response.text().await.unwrap_or_default();
            return Err(status_error(status, &text, retry_after));
        }

        Ok(into_chunk_stream(response.bytes_stream()))
    }
}

fn is_retryable(request: &GenerateRequest) -> bool {
    !request
        .messages
        .iter()
        .any(|message| message.role == Role::Tool)
}

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

fn status_error(
    status: reqwest::StatusCode,
    body: &str,
    retry_after: Option<Duration>,
) -> ModelError {
    let detail = parse_error_detail(body);

    match status.as_u16() {
        401 | 403 => ModelError::AuthFailed,
        408 => ModelError::Timeout,
        429 => ModelError::RateLimited { retry_after },
        code if (500..600).contains(&code) => ModelError::Unavailable(detail),
        code if (400..500).contains(&code) => ModelError::InvalidRequest(detail),
        code => ModelError::Rejected(format!("unexpected status {code}: {detail}")),
    }
}

fn parse_error_detail(body: &str) -> String {
    #[derive(serde::Deserialize)]
    struct ErrorWrapper {
        error: Option<ErrorDetail>,
    }
    #[derive(serde::Deserialize)]
    struct ErrorDetail {
        message: Option<String>,
    }

    if let Ok(wrapper) = serde_json::from_str::<ErrorWrapper>(body)
        && let Some(msg) = wrapper.error.and_then(|e| e.message)
    {
        return msg;
    }

    body.chars().take(200).collect()
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
