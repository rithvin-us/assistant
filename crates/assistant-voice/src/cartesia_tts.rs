//! Cartesia Text-to-Speech (TTS) implementation.

use crate::traits::{AudioEncoding, TextToSpeechProvider, TtsAudioChunk, TtsRequest, VoiceError};
use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc;

pub struct CartesiaTtsProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    default_voice_id: String,
    endpoint: String,
}

#[derive(Debug, Serialize)]
struct CartesiaTtsPayload<'a> {
    model_id: &'a str,
    transcript: &'a str,
    voice: CartesiaVoiceSpec<'a>,
    output_format: CartesiaOutputFormat<'a>,
}

#[derive(Debug, Serialize)]
struct CartesiaVoiceSpec<'a> {
    mode: &'a str,
    id: &'a str,
}

#[derive(Debug, Serialize)]
struct CartesiaOutputFormat<'a> {
    container: &'a str,
    encoding: &'a str,
    sample_rate: u32,
}

impl CartesiaTtsProvider {
    pub fn new(api_key: String, model: Option<String>, default_voice_id: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model: model.unwrap_or_else(|| "sonic-2".to_string()),
            default_voice_id: default_voice_id
                .unwrap_or_else(|| "a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
            endpoint: "https://api.cartesia.ai/tts/bytes".to_string(),
        }
    }

    pub fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint = endpoint;
        self
    }
}

#[async_trait]
impl TextToSpeechProvider for CartesiaTtsProvider {
    async fn synthesize(&self, request: TtsRequest) -> Result<Vec<u8>, VoiceError> {
        if self.api_key.trim().is_empty() {
            return Err(VoiceError::Unconfigured(
                "CARTESIA_API_KEY is not configured".into(),
            ));
        }
        if request.text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let voice_id = request
            .voice_id
            .as_deref()
            .unwrap_or(&self.default_voice_id);

        let model_id = request.model.as_deref().unwrap_or(&self.model);

        let (container, encoding) = match request.encoding.unwrap_or(AudioEncoding::Wav) {
            AudioEncoding::Wav => ("wav", "pcm_s16le"),
            AudioEncoding::Mp3 => ("mp3", "mp3"),
            AudioEncoding::PcmS16Le => ("raw", "pcm_s16le"),
            AudioEncoding::Webm => ("wav", "pcm_s16le"),
        };

        let sample_rate = request.sample_rate.unwrap_or(24000);

        let payload = CartesiaTtsPayload {
            model_id,
            transcript: &request.text,
            voice: CartesiaVoiceSpec {
                mode: "id",
                id: voice_id,
            },
            output_format: CartesiaOutputFormat {
                container,
                encoding,
                sample_rate,
            },
        };

        let res = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Cartesia-Version", "2024-06-10")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| VoiceError::Network(e.to_string()))?;

        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(VoiceError::AuthenticationFailed(
                "Invalid Cartesia API key".into(),
            ));
        }

        if !status.is_success() {
            let err_text = res.text().await.unwrap_or_default();
            return Err(VoiceError::Api {
                code: status.to_string(),
                message: err_text,
            });
        }

        let bytes = res
            .bytes()
            .await
            .map_err(|e| VoiceError::Network(e.to_string()))?;

        Ok(bytes.to_vec())
    }

    async fn synthesize_stream(
        &self,
        request: TtsRequest,
    ) -> Result<mpsc::Receiver<Result<TtsAudioChunk, VoiceError>>, VoiceError> {
        let (tx, rx) = mpsc::channel(16);
        let audio_bytes = self.synthesize(request).await?;

        tokio::spawn(async move {
            if !audio_bytes.is_empty() {
                // Split audio into streaming chunks of 8KB
                for chunk in audio_bytes.chunks(8192) {
                    let _ = tx
                        .send(Ok(TtsAudioChunk {
                            audio_bytes: chunk.to_vec(),
                            is_final: false,
                        }))
                        .await;
                }
            }
            let _ = tx
                .send(Ok(TtsAudioChunk {
                    audio_bytes: Vec::new(),
                    is_final: true,
                }))
                .await;
        });

        Ok(rx)
    }
}
