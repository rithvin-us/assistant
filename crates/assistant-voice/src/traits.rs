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
    #[error("Voice request timed out after {0:?}")]
    Timeout(std::time::Duration),
}

impl VoiceError {
    /// A stable, machine-readable code for this failure.
    ///
    /// The client needs to tell "your session expired" from "the network
    /// dropped" from "Cartesia rejected it" so it can offer the right recovery.
    /// Everything used to collapse into `stt_error`/`tts_error` with a bare
    /// message, which left the UI nothing to act on.
    ///
    /// These names are part of the wire contract -- changing one changes client
    /// behaviour. They deliberately carry no provider detail or secret.
    pub fn code(&self) -> &'static str {
        match self {
            VoiceError::Unconfigured(_) => "voice_unconfigured",
            VoiceError::AuthenticationFailed(_) => "voice_provider_auth_failed",
            VoiceError::Network(_) => "voice_network_error",
            VoiceError::AudioEncoding(_) => "voice_audio_encoding_error",
            VoiceError::Api { .. } => "voice_provider_error",
            VoiceError::Cancelled => "voice_cancelled",
            VoiceError::Timeout(_) => "voice_timeout",
        }
    }

    /// Whether retrying the identical request could plausibly succeed.
    ///
    /// A bad model id or a rejected key will fail identically forever; a
    /// timeout or a dropped connection may not.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            VoiceError::Network(_) | VoiceError::Timeout(_) | VoiceError::Api { .. }
        )
    }
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

/// Default ceiling on a single provider call.
///
/// Both providers previously used a bare `reqwest::Client::new()`, which has no
/// request timeout, so a hung STT or TTS call hung the voice turn forever with
/// no way to recover. A voice turn the user is waiting on has to fail fast.
pub const DEFAULT_PROVIDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Cartesia API version, sent on every request.
///
/// Cartesia dates its API and rejects unknown values, so this is a hard
/// compatibility pin rather than a nicety. It was `2024-06-10`, which the
/// current API refuses; verified against the live endpoint before changing.
pub const CARTESIA_API_VERSION: &str = "2026-08-14";

/// Default Cartesia STT model. Must be in the `ink-whisper` family -- the
/// previous `ink-en-us` is not a real model id and returned 400 invalid model.
pub const DEFAULT_STT_MODEL: &str = "ink-whisper";

/// Default Cartesia TTS model. `sonic-english` and `sonic-2` are both gone from
/// the current model list.
pub const DEFAULT_TTS_MODEL: &str = "sonic-3.6";

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

/// Classifies a transport failure, keeping a timeout distinct from a generic
/// network fault so the client can decide whether retrying is worthwhile.
pub fn map_reqwest_error(e: reqwest::Error) -> VoiceError {
    if e.is_timeout() {
        VoiceError::Timeout(DEFAULT_PROVIDER_TIMEOUT)
    } else {
        VoiceError::Network(e.to_string())
    }
}
