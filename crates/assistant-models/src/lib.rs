//! The seam between the assistant and whatever produces tokens.
//!
//! Nothing in this crate names Claude, Gemini or any other vendor. Provider
//! implementations will live in sibling modules behind cargo features so that
//! `assistant-core` never links a provider it does not use. See
//! docs/DECISIONS.md ADR-0003.

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub max_output_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResponse {
    pub text: String,
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
