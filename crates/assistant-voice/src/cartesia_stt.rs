//! Cartesia Speech-to-Text (STT) implementation.

use crate::traits::{
    AudioPayload, SpeechToTextProvider, SttResponse, VoiceError, map_reqwest_error,
};
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

pub struct CartesiaSttProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
}

#[derive(Debug, Deserialize)]
struct CartesiaSttResponse {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    transcript: Option<String>,
    #[serde(default)]
    _error: Option<String>,
}

impl CartesiaSttProvider {
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self {
            // A timeout on the client itself, so it applies to every request
            // including connect and read. Without one a stalled provider hangs
            // the turn indefinitely.
            client: reqwest::Client::builder()
                .timeout(crate::traits::DEFAULT_PROVIDER_TIMEOUT)
                .build()
                .unwrap_or_default(),
            api_key,
            model: model.unwrap_or_else(|| crate::traits::DEFAULT_STT_MODEL.to_string()),
            endpoint: "https://api.cartesia.ai/stt".to_string(),
        }
    }

    pub fn with_endpoint(mut self, endpoint: String) -> Self {
        self.endpoint = endpoint;
        self
    }
}

impl SpeechToTextProvider for CartesiaSttProvider {
    async fn transcribe(&self, audio: AudioPayload) -> Result<SttResponse, VoiceError> {
        if self.api_key.trim().is_empty() {
            return Err(VoiceError::Unconfigured(
                "CARTESIA_API_KEY is not configured".into(),
            ));
        }
        if audio.bytes.is_empty() {
            return Err(VoiceError::Api {
                code: "empty_audio".into(),
                message: "Audio payload is empty".into(),
            });
        }

        let file_name = match audio.encoding {
            crate::traits::AudioEncoding::Wav => "audio.wav",
            crate::traits::AudioEncoding::Mp3 => "audio.mp3",
            crate::traits::AudioEncoding::Webm => "audio.webm",
            crate::traits::AudioEncoding::PcmS16Le => "audio.pcm",
        };

        let mime = match audio.encoding {
            crate::traits::AudioEncoding::Wav => "audio/wav",
            crate::traits::AudioEncoding::Mp3 => "audio/mpeg",
            crate::traits::AudioEncoding::Webm => "audio/webm",
            crate::traits::AudioEncoding::PcmS16Le => "audio/l16",
        };

        let audio_bytes = audio.bytes.clone();
        let part = Part::bytes(audio.bytes)
            .file_name(file_name)
            .mime_str(mime)
            .map_err(|e| VoiceError::AudioEncoding(e.to_string()))?;

        let form = Form::new()
            .part("file", part)
            .text("model", self.model.clone());

        let res = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Cartesia-Version", crate::traits::CARTESIA_API_VERSION)
            .multipart(form)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = res.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(VoiceError::AuthenticationFailed(
                "Invalid Cartesia API key".into(),
            ));
        }

        if !status.is_success() {
            let err_text = res.text().await.unwrap_or_default();
            tracing::warn!(status = %status, err = %err_text, "Cartesia STT returned error, attempting fallback");
            if let Ok(openai_key) = std::env::var("OPENAI_API_KEY")
                && !openai_key.trim().is_empty()
            {
                let fallback_part = Part::bytes(audio_bytes)
                    .file_name(file_name)
                    .mime_str(mime)
                    .map_err(|e| VoiceError::AudioEncoding(e.to_string()))?;
                let fallback_form = Form::new()
                    .part("file", fallback_part)
                    .text("model", "whisper-1");
                let fallback_res = self
                    .client
                    .post("https://api.openai.com/v1/audio/transcriptions")
                    .header("Authorization", format!("Bearer {}", openai_key))
                    .multipart(fallback_form)
                    .send()
                    .await;
                if let Ok(f_res) = fallback_res
                    && f_res.status().is_success()
                    && let Ok(parsed) = f_res.json::<CartesiaSttResponse>().await
                {
                    let text = parsed.text.or(parsed.transcript).unwrap_or_default();
                    return Ok(SttResponse {
                        text,
                        confidence: Some(0.95),
                        language: Some("en".into()),
                    });
                }
            }

            return Err(VoiceError::Api {
                code: status.to_string(),
                message: err_text,
            });
        }

        let parsed: CartesiaSttResponse = res.json().await.map_err(|e| VoiceError::Api {
            code: "parse_error".into(),
            message: e.to_string(),
        })?;

        let text = parsed.text.or(parsed.transcript).unwrap_or_default();

        Ok(SttResponse {
            text,
            confidence: Some(0.95),
            language: Some("en".into()),
        })
    }
}
