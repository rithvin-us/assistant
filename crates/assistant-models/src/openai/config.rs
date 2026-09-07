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
    pub const DEFAULT_BASE_URL: &'static str = "https://generativelanguage.googleapis.com/v1beta/openai";
    pub const DEFAULT_MODEL: &'static str = "gemini-flash-latest";
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
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") || base.ends_with("/openai") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_completions_url_default() {
        let cfg = OpenAIConfig::new("test-key");
        assert_eq!(
            cfg.chat_completions_url(),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }

    #[test]
    fn test_chat_completions_url_gemini_openai() {
        let mut cfg = OpenAIConfig::new("test-key");
        cfg.base_url = "https://generativelanguage.googleapis.com/v1beta/openai".to_string();
        assert_eq!(
            cfg.chat_completions_url(),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }

    #[test]
    fn test_chat_completions_url_with_v1() {
        let mut cfg = OpenAIConfig::new("test-key");
        cfg.base_url = "https://api.openai.com/v1".to_string();
        assert_eq!(
            cfg.chat_completions_url(),
            "https://api.openai.com/v1/chat/completions"
        );
    }
}

