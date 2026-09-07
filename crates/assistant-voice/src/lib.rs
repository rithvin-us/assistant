//! Assistant Voice Crate: Cartesia STT + TTS integration, abstractions & state machine.

pub mod cartesia_stt;
pub mod cartesia_tts;
pub mod fake_providers;
pub mod state_machine;
pub mod traits;

pub use cartesia_stt::CartesiaSttProvider;
pub use cartesia_tts::CartesiaTtsProvider;
pub use fake_providers::{FakeSpeechToTextProvider, FakeTextToSpeechProvider};
pub use state_machine::{StateMachineError, VoiceState, VoiceStateMachine};
pub use traits::{
    AudioEncoding, AudioPayload, SpeechToTextProvider, SttPartial, SttResponse,
    TextToSpeechProvider, TtsAudioChunk, TtsRequest, VoiceError,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_state_machine_valid_flow() {
        let mut sm = VoiceStateMachine::new();
        assert_eq!(sm.state(), VoiceState::Idle);

        assert!(sm.transition_to(VoiceState::Listening).is_ok());
        assert_eq!(sm.state(), VoiceState::Listening);

        assert!(sm.transition_to(VoiceState::Transcribing).is_ok());
        assert_eq!(sm.state(), VoiceState::Transcribing);

        assert!(sm.transition_to(VoiceState::Thinking).is_ok());
        assert_eq!(sm.state(), VoiceState::Thinking);

        assert!(sm.transition_to(VoiceState::Speaking).is_ok());
        assert_eq!(sm.state(), VoiceState::Speaking);

        assert!(sm.transition_to(VoiceState::Idle).is_ok());
        assert_eq!(sm.state(), VoiceState::Idle);
    }

    #[test]
    fn test_voice_state_machine_interruption() {
        let mut sm = VoiceStateMachine::new();
        sm.transition_to(VoiceState::Listening).unwrap();
        sm.transition_to(VoiceState::Transcribing).unwrap();
        sm.transition_to(VoiceState::Thinking).unwrap();
        sm.transition_to(VoiceState::Speaking).unwrap();

        // User interrupts speaking
        assert!(sm.transition_to(VoiceState::Interrupted).is_ok());
        assert_eq!(sm.state(), VoiceState::Interrupted);

        assert!(sm.transition_to(VoiceState::Idle).is_ok());
    }

    #[test]
    fn test_voice_state_machine_invalid_transition() {
        let mut sm = VoiceStateMachine::new();
        // Cannot jump directly from Idle to Speaking
        assert!(sm.transition_to(VoiceState::Speaking).is_err());
    }

    #[tokio::test]
    async fn test_fake_stt_provider() {
        let provider = FakeSpeechToTextProvider::default();
        let payload = AudioPayload {
            bytes: vec![1, 2, 3],
            encoding: AudioEncoding::Wav,
            sample_rate: 16000,
            channels: 1,
        };
        let res = provider.transcribe(payload).await.unwrap();
        assert_eq!(res.text, "What should I do today?");
    }

    #[tokio::test]
    async fn test_fake_tts_provider() {
        let provider = FakeTextToSpeechProvider::default();
        let req = TtsRequest {
            text: "Hello".to_string(),
            voice_id: None,
            model: None,
            encoding: Some(AudioEncoding::Wav),
            sample_rate: Some(24000),
        };
        let bytes = provider.synthesize(req).await.unwrap();
        assert_eq!(bytes, vec![0, 1, 2, 3, 4, 5, 6, 7]);
    }
}
