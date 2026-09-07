//! Provider abstractions and domain models for Voice (STT & TTS).

use serde::{Deserialize, Serialize};
use std::future::Future;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum VoiceError {
    #[error("Voice service unconfigured: {0}")]
    Unconfigured(String),
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("Network failure: {0}")]
    Network(String),
    #[error("Audio encoding error: {0}")]
    AudioEncoding(String),
    #[error("API error ({code}): {message}")]
    Api { code: String, message: String },
    #[error("Voice request cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioEncoding {
    PcmS16Le,
    Wav,
    Mp3,
    Webm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPayload {
    pub bytes: Vec<u8>,
    pub encoding: AudioEncoding,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttResponse {
    pub text: String,
    pub confidence: Option<f32>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttPartial {
    pub text: String,
    pub is_final: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub voice_id: Option<String>,
    pub model: Option<String>,
    pub encoding: Option<AudioEncoding>,
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsAudioChunk {
    pub audio_bytes: Vec<u8>,
    pub is_final: bool,
}

/// Abstract Speech-to-Text Provider.
///
/// The futures are desugared rather than written as `async fn` so the `Send`
/// bound is part of the trait: the conversation and voice WebSocket handlers
/// drive these providers inside `tokio::spawn`, which requires it.
pub trait SpeechToTextProvider: Send + Sync {
    fn transcribe(
        &self,
        audio: AudioPayload,
    ) -> impl Future<Output = Result<SttResponse, VoiceError>> + Send;
}

/// Abstract Text-to-Speech Provider.
pub trait TextToSpeechProvider: Send + Sync {
    fn synthesize(
        &self,
        request: TtsRequest,
    ) -> impl Future<Output = Result<Vec<u8>, VoiceError>> + Send;

    fn synthesize_stream(
        &self,
        request: TtsRequest,
    ) -> impl Future<Output = Result<mpsc::Receiver<Result<TtsAudioChunk, VoiceError>>, VoiceError>> + Send;
}
