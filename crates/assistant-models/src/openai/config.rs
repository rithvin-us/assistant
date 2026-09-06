//! OpenAI provider configuration.

use std::{fmt, time::Duration};

pub struct OpenAIConfig {
    api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_output_tokens: u32,
    pub temperature: Option<f32>,
    pub timeout: Duration,
    pub streaming: bool,
    pub max_transport_retries: u32,
}

impl OpenAIConfig {
    pub const DEFAULT_BASE_URL: &'static str = "https://api.openai.com";
    pub const DEFAULT_MODEL: &'static str = "gpt-4o-mini";
    pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
    pub const DEFAULT_MAX_TRANSPORT_RETRIES: u32 = 2;

    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: Self::DEFAULT_BASE_URL.to_string(),
            model: Self::DEFAULT_MODEL.to_string(),
            max_output_tokens: Self::DEFAULT_MAX_OUTPUT_TOKENS,
            temperature: None,
            timeout: Self::DEFAULT_TIMEOUT,
            streaming: true,
            max_transport_retries: Self::DEFAULT_MAX_TRANSPORT_RETRIES,
        }
    }

    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    pub fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

impl fmt::Debug for OpenAIConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAIConfig")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("temperature", &self.temperature)
            .field("timeout", &self.timeout)
            .field("streaming", &self.streaming)
            .field("max_transport_retries", &self.max_transport_retries)
            .finish()
    }
}
