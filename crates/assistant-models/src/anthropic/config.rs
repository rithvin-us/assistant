//! Provider configuration.
//!
//! Everything the provider needs is on one struct, supplied by the caller. The
//! provider reads no environment variable of its own: the server's
//! configuration layer owns that, so there is a single place to audit for
//! secret handling. See docs/DECISIONS.md ADR-0020.

use std::{fmt, time::Duration};

use serde::Serialize;

/// How much reasoning effort to ask for.
///
/// Serialised inside `output_config`. Lower effort means fewer thinking tokens
/// and a faster first token, which is what a conversational turn wants; a
/// deployment that needs deeper reasoning raises it in configuration rather
/// than in code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl Effort {
    /// Parses the value a configuration layer read from the environment.
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            _ => return None,
        })
    }
}

/// Everything that varies between deployments of the Anthropic provider.
///
/// The model name appears here and nowhere else, so changing it is a
/// configuration change rather than a code search.
pub struct AnthropicConfig {
    /// Server-only credential. Private, never serialised, redacted in `Debug`.
    api_key: String,
    /// Overridden only by tests, which point it at a local socket.
    pub base_url: String,
    pub model: String,
    pub max_output_tokens: u32,
    /// Left `None` by default: the current Claude models reject `temperature`
    /// outright, so sending one would fail every request. It stays on the
    /// struct because a future or alternate model may accept it.
    pub temperature: Option<f32>,
    pub effort: Option<Effort>,
    /// Applies to a whole non-streaming request, and between reads of a
    /// streaming one -- a stream that is still producing tokens is not late.
    pub timeout: Duration,
    pub streaming: bool,
    /// Bounded retry for transport failures only. See
    /// [`super::AnthropicModelProvider::send`] for what is and is not retried.
    pub max_transport_retries: u32,
}

impl AnthropicConfig {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.anthropic.com";
    pub const DEFAULT_MODEL: &'static str = "claude-opus-5";

    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: Self::DEFAULT_BASE_URL.to_string(),
            model: Self::DEFAULT_MODEL.to_string(),
            max_output_tokens: 4096,
            temperature: None,
            // A conversational turn is latency-critical and rarely needs deep
            // reasoning. Thinking stays on -- disabling it on current models
            // makes them occasionally narrate a tool call instead of emitting
            // one -- and the depth is turned down instead.
            effort: Some(Effort::Low),
            timeout: Duration::from_secs(60),
            streaming: true,
            max_transport_retries: 1,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_max_output_tokens(mut self, tokens: u32) -> Self {
        self.max_output_tokens = tokens;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_effort(mut self, effort: Option<Effort>) -> Self {
        self.effort = effort;
        self
    }

    pub fn with_streaming(mut self, streaming: bool) -> Self {
        self.streaming = streaming;
        self
    }

    pub fn with_max_transport_retries(mut self, retries: u32) -> Self {
        self.max_transport_retries = retries;
        self
    }

    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    pub(crate) fn messages_url(&self) -> String {
        format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
    }
}

/// Redacts the credential. Adding a secret field without adding it here as
/// `<redacted>` is the one mistake this impl exists to prevent.
impl fmt::Debug for AnthropicConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("temperature", &self.temperature)
            .field("effort", &self.effort)
            .field("timeout", &self.timeout)
            .field("streaming", &self.streaming)
            .field("max_transport_retries", &self.max_transport_retries)
            .finish()
    }
}
