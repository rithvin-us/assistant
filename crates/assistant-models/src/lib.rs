//! The seam between the assistant and whatever produces tokens.
//!
//! Nothing in this crate names Claude, Gemini or any other vendor. Provider
//! implementations will live in sibling modules behind cargo features so that
//! `assistant-core` never links a provider it does not use. See
//! docs/DECISIONS.md ADR-0003.

#[cfg(feature = "mock")]
pub mod mock;

use std::pin::Pin;

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

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("provider does not support {0:?}")]
    UnsupportedCapability(Capability),
    #[error("request rejected by provider: {0}")]
    Rejected(String),
    #[error("provider transport failure: {0}")]
    Transport(String),
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
