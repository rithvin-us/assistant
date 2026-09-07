//! Fake Speech-to-Text and Text-to-Speech providers for unit testing.

use crate::traits::{
    AudioPayload, SpeechToTextProvider, SttResponse, TextToSpeechProvider, TtsAudioChunk,
    TtsRequest, VoiceError,
};
use async_trait::async_trait;
use tokio::sync::mpsc;

pub struct FakeSpeechToTextProvider {
    pub mock_text: String,
    pub should_fail: bool,
}

impl Default for FakeSpeechToTextProvider {
    fn default() -> Self {
        Self {
            mock_text: "What should I do today?".to_string(),
            should_fail: false,
        }
    }
}

#[async_trait]
impl SpeechToTextProvider for FakeSpeechToTextProvider {
    async fn transcribe(&self, _audio: AudioPayload) -> Result<SttResponse, VoiceError> {
        if self.should_fail {
            return Err(VoiceError::Api {
                code: "mock_failure".into(),
                message: "Mock STT failure".into(),
            });
        }
        Ok(SttResponse {
            text: self.mock_text.clone(),
            confidence: Some(0.99),
            language: Some("en".into()),
        })
    }
}

pub struct FakeTextToSpeechProvider {
    pub mock_audio: Vec<u8>,
    pub should_fail: bool,
}

impl Default for FakeTextToSpeechProvider {
    fn default() -> Self {
        Self {
            mock_audio: vec![0, 1, 2, 3, 4, 5, 6, 7],
            should_fail: false,
        }
    }
}

#[async_trait]
impl TextToSpeechProvider for FakeTextToSpeechProvider {
    async fn synthesize(&self, request: TtsRequest) -> Result<Vec<u8>, VoiceError> {
        if self.should_fail {
            return Err(VoiceError::Api {
                code: "mock_failure".into(),
                message: "Mock TTS failure".into(),
            });
        }
        if request.text.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.mock_audio.clone())
    }

    async fn synthesize_stream(
        &self,
        _request: TtsRequest,
    ) -> Result<mpsc::Receiver<Result<TtsAudioChunk, VoiceError>>, VoiceError> {
        if self.should_fail {
            return Err(VoiceError::Api {
                code: "mock_failure".into(),
                message: "Mock TTS stream failure".into(),
            });
        }
        let (tx, rx) = mpsc::channel(4);
        let audio = self.mock_audio.clone();

        tokio::spawn(async move {
            let _ = tx
                .send(Ok(TtsAudioChunk {
                    audio_bytes: audio,
                    is_final: true,
                }))
                .await;
        });

        Ok(rx)
    }
}
