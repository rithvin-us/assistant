//! The seam between the assistant and whatever produces tokens.
//!
//! The vocabulary in this module names no vendor. Provider implementations live
//! in sibling modules behind cargo features, so `assistant-core` -- which
//! depends on this crate with no features -- never links a provider, and never
//! sees a provider-specific type. See docs/DECISIONS.md ADR-0003 and ADR-0018.

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "openai")]
pub mod openai;

use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

/// What a provider can do. The router uses this to pick a model for a workload
/// instead of hard-coding provider names at call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Single-shot completion.
    Generate,
    /// Incremental token streaming, required for low-latency voice.
    Stream,
    /// Output constrained to a caller-supplied JSON schema.
    StructuredOutput,
    /// Native tool/function calling.
    ToolUse,
    /// Image or document input.
    Multimodal,
    /// Bidirectional realtime audio.
    RealtimeAudio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    /// The result of a tool the assistant called. Carries `tool_call_id` so a
    /// provider can link it back to the call it answers.
    Tool,
}

/// One entry in the conversation sent to a provider.
///
/// Tool calls and tool results are first-class rather than prose stuffed into
/// `content`, because a provider that supports native tool use needs them
/// structured, and flattening them here would make that impossible to recover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls proposed by this assistant turn. Empty for every other role.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<assistant_tools::ToolCall>,
    /// For [`Role::Tool`], the id of the call this message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self::plain(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, content)
    }

    /// An assistant turn that proposed tool calls.
    pub fn assistant_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<assistant_tools::ToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// The result of one tool call, fed back to the model.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub model: ModelId,
    pub system_prompt: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<assistant_tools::ToolSpec>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<assistant_tools::ToolCall>,
    pub usage: Usage,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// One increment of a streamed response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    ToolCall(assistant_tools::ToolCall),
    Done(Usage),
}

/// What went wrong at the provider boundary.
///
/// Two rules govern this type, and they are the reason it is an enum rather
/// than a string.
///
/// * **`Display` is for logs, not for users.** Several variants carry provider
///   detail so a failure can be diagnosed; that detail reaches the log through
///   the error source chain and never reaches the wire. What the user is told
///   comes from [`ModelError::user_message`], which names no vendor and quotes
///   nothing the provider said.
/// * **`code` is stable.** The transport maps a failure onto a wire frame from
///   this discriminant, so adding a variant without adding a code is a compile
///   error.
///
/// Nothing here ever carries an API key or a request header: the provider
/// constructs these from a status line and a parsed error body, never from the
/// request it sent.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("provider does not support {0:?}")]
    UnsupportedCapability(Capability),

    /// Credentials were missing, malformed or rejected.
    #[error("provider rejected the credentials")]
    AuthFailed,

    /// The deployment is being throttled. `retry_after` is the provider's own
    /// advice, when it gave any; the core does not act on it automatically.
    #[error("provider is rate limiting this deployment")]
    RateLimited { retry_after: Option<Duration> },

    /// No response, or no further bytes, within the configured window.
    #[error("provider did not respond within the configured timeout")]
    Timeout,

    /// The request this build constructed was not acceptable. Almost always a
    /// bug here rather than a user problem, so the detail matters in the log.
    #[error("provider rejected the request: {0}")]
    InvalidRequest(String),

    /// The provider is down, overloaded, or unreachable.
    #[error("provider is unavailable: {0}")]
    Unavailable(String),

    /// A response arrived that this build cannot read. Distinct from
    /// `InvalidRequest`: the request was fine and the answer was not.
    #[error("provider returned a response this build cannot parse: {0}")]
    MalformedResponse(String),

    /// The provider's safety systems declined to answer. Not a fault, and not
    /// something a retry fixes.
    #[error("provider declined to answer{}", match .category {
        Some(category) => format!(" ({category})"),
        None => String::new(),
    })]
    Refused { category: Option<String> },

    #[error("request rejected by provider: {0}")]
    Rejected(String),

    #[error("provider transport failure: {0}")]
    Transport(String),
}

impl ModelError {
    /// Stable discriminant for the wire and for metrics.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedCapability(_) => "provider_unsupported_capability",
            Self::AuthFailed => "provider_auth_failed",
            Self::RateLimited { .. } => "provider_rate_limited",
            Self::Timeout => "provider_timeout",
            Self::InvalidRequest(_) => "provider_invalid_request",
            Self::Unavailable(_) => "provider_unavailable",
            Self::MalformedResponse(_) => "provider_error",
            Self::Refused { .. } => "provider_refused",
            Self::Rejected(_) => "provider_error",
            Self::Transport(_) => "provider_unavailable",
        }
    }

    /// Text that is safe to show a user.
    ///
    /// Deliberately quotes nothing the provider returned and names no vendor: a
    /// provider error body is untrusted text that could contain anything,
    /// including account identifiers.
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::UnsupportedCapability(_) => {
                "The configured language model cannot do what this turn needs."
            }
            Self::AuthFailed => {
                "The assistant is not configured correctly and could not reach its language model."
            }
            Self::RateLimited { .. } => "The assistant is busy right now. Try again in a moment.",
            Self::Timeout => "The assistant took too long to answer. Try again.",
            Self::InvalidRequest(_) | Self::MalformedResponse(_) | Self::Rejected(_) => {
                "The assistant could not complete that turn."
            }
            Self::Unavailable(_) | Self::Transport(_) => {
                "The assistant could not reach its language model. Try again."
            }
            Self::Refused { .. } => "The assistant declined to answer that.",
        }
    }

    /// Whether retrying the identical request could plausibly succeed.
    ///
    /// Used only by a provider's own bounded transport retry. Nothing above the
    /// provider retries a model call, because a turn may already have run tools
    /// by the time it fails.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Unavailable(_) | Self::Transport(_)
        )
    }
}

pub type ChunkStream = Pin<Box<dyn Stream<Item = Result<StreamChunk, ModelError>> + Send>>;

/// Implemented once per provider.
///
/// `stream` is not a convenience over `generate`: streaming is a product
/// requirement for voice latency, so a provider that cannot stream must say so
/// via [`Self::capabilities`] rather than silently buffering.
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;

    fn capabilities(&self) -> &[Capability];

    fn supports(&self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }

    async fn generate(&self, request: GenerateRequest) -> Result<GenerateResponse, ModelError>;

    async fn stream(&self, request: GenerateRequest) -> Result<ChunkStream, ModelError>;
}
