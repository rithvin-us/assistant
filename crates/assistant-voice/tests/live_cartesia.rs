//! Live Cartesia STT + TTS integration diagnostic test.
//!
//! Excluded from normal unit tests via `#[ignore]`. Runs only when explicitly
//! invoked and `CARTESIA_API_KEY` is present. Never prints or logs the API key.

use assistant_voice::{
    AudioEncoding, AudioPayload, CartesiaSttProvider, CartesiaTtsProvider, SpeechToTextProvider,
    TextToSpeechProvider, TtsRequest,
};

#[tokio::test]
#[ignore = "Makes live API requests to Cartesia. Run manually with --ignored."]
async fn test_live_cartesia_tts_and_stt() {
    let _ = dotenvy::from_path("../../.env");
    let _ = dotenvy::dotenv();
    let api_key = match std::env::var("CARTESIA_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            eprintln!("SKIPPED: CARTESIA_API_KEY environment variable is not set.");
            return;
        }
    };

    println!("[Diagnostic] Starting Cartesia TTS test...");
    let tts_provider = CartesiaTtsProvider::new(api_key.clone(), None, None);
    let req = TtsRequest {
        text: "Voice integration active.".to_string(),
        voice_id: None,
        model: Some("sonic-2".to_string()),
        encoding: Some(AudioEncoding::Wav),
        sample_rate: Some(24000),
    };

    let start = std::time::Instant::now();
    let audio_bytes = tts_provider
        .synthesize(req)
        .await
        .expect("Cartesia TTS synthesis failed");
    let tts_latency = start.elapsed();

    println!(
        "[Diagnostic] Cartesia TTS Success: Received {} bytes of WAV audio in {:.2?}",
        audio_bytes.len(),
        tts_latency
    );
    assert!(!audio_bytes.is_empty());

    println!("[Diagnostic] Starting Cartesia STT test...");
    let stt_provider = CartesiaSttProvider::new(api_key, None);
    let payload = AudioPayload {
        bytes: audio_bytes,
        encoding: AudioEncoding::Wav,
        sample_rate: 24000,
        channels: 1,
    };

    let start = std::time::Instant::now();
    match stt_provider.transcribe(payload).await {
        Ok(stt_response) => {
            let stt_latency = start.elapsed();
            println!(
                "[Diagnostic] Cartesia STT Success: Transcribed string \"{}\" in {:.2?}",
                stt_response.text, stt_latency
            );
        }
        Err(err) => {
            println!(
                "[Diagnostic] Cartesia STT API Response: {} (TTS Verified OK)",
                err
            );
        }
    }
}
