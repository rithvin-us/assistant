//! Voice endpoints: Cartesia STT, TTS, and Diagnostic.

use assistant_auth::Principal;
use assistant_protocol::ApiError;
use assistant_voice::{
    AudioEncoding, AudioPayload, CartesiaSttProvider, CartesiaTtsProvider, SpeechToTextProvider,
    TextToSpeechProvider, TtsRequest,
};
use axum::{
    Extension,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::{rate_limit::RateLimited, state::SharedState};

/// Turns a refusal into the response, including `Retry-After` so a client can
/// back off rather than hammering.
fn too_many_requests(limited: RateLimited) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ApiError {
            code: "rate_limited".into(),
            message: format!(
                "Too many voice requests. Try again in {} seconds.",
                limited.retry_after_secs
            ),
        }),
    )
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoiceTranscribeResponse {
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct VoiceSpeakRequest {
    pub text: String,
    pub voice_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoiceSpeakResponse {
    pub audio_base64: String,
    pub encoding: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VoiceDiagnosticResponse {
    pub cartesia_configured: bool,
    pub stt_model: String,
    pub tts_model: String,
    pub tts_voice_id: String,
    pub status: String,
}

/// Ceiling on an uploaded audio body.
///
/// There was no limit at all: any body size was forwarded straight to the
/// provider, so one request could spend an unbounded amount of money and hold
/// an unbounded amount of memory.
pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;

/// Ceiling on text submitted for synthesis, for the same reason.
const MAX_TTS_CHARS: usize = 4_000;

pub async fn transcribe(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<VoiceTranscribeResponse>, (StatusCode, Json<ApiError>)> {
    // Metered per principal, before any work is done on the request.
    state
        .voice_rate_limiter
        .check(principal.user_id, "stt")
        .map_err(too_many_requests)?;

    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "empty_audio".into(),
                message: "Audio payload cannot be empty.".into(),
            }),
        ));
    }

    // An unbounded audio body is a cheap way to make the server buy a very
    // expensive provider call, or to exhaust its memory. A minute of speech is
    // far below this; anything above it is not someone talking to an assistant.
    if body.len() > MAX_AUDIO_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ApiError {
                code: "audio_too_large".into(),
                message: format!(
                    "Audio payload is larger than the {} MB limit.",
                    MAX_AUDIO_BYTES / (1024 * 1024)
                ),
            }),
        ));
    }

    let api_key = state.cartesia_api_key.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                code: "cartesia_not_configured".into(),
                message: "CARTESIA_API_KEY is not configured on the server.".into(),
            }),
        )
    })?;

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/wav");

    let encoding = if content_type.contains("mp3") {
        AudioEncoding::Mp3
    } else if content_type.contains("webm") {
        AudioEncoding::Webm
    } else {
        AudioEncoding::Wav
    };

    let stt = CartesiaSttProvider::new(api_key.to_string(), Some(state.cartesia_stt_model.clone()));
    let payload = AudioPayload {
        bytes: body.to_vec(),
        encoding,
        sample_rate: 24000,
        channels: 1,
    };

    let res = stt.transcribe(payload).await.map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                code: "stt_error".into(),
                message: err.to_string(),
            }),
        )
    })?;

    Ok(Json(VoiceTranscribeResponse { text: res.text }))
}

pub async fn speak(
    State(state): State<SharedState>,
    Extension(principal): Extension<Principal>,
    Json(payload): Json<VoiceSpeakRequest>,
) -> Result<Json<VoiceSpeakResponse>, (StatusCode, Json<ApiError>)> {
    state
        .voice_rate_limiter
        .check(principal.user_id, "tts")
        .map_err(too_many_requests)?;

    if payload.text.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "empty_text".into(),
                message: "Text string cannot be empty.".into(),
            }),
        ));
    }

    // Synthesis is billed by length, so unbounded text is an unbounded bill.
    if payload.text.chars().count() > MAX_TTS_CHARS {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ApiError {
                code: "text_too_long".into(),
                message: format!("Text is longer than the {MAX_TTS_CHARS} character limit."),
            }),
        ));
    }

    let audio_res = if let Some(ref api_key) = state.cartesia_api_key {
        let tts = CartesiaTtsProvider::new(
            api_key.to_string(),
            Some(state.cartesia_tts_model.clone()),
            Some(
                payload
                    .voice_id
                    .clone()
                    .unwrap_or_else(|| state.cartesia_tts_voice_id.clone()),
            ),
        );

        let req = TtsRequest {
            text: payload.text.clone(),
            voice_id: None,
            model: None,
            encoding: Some(AudioEncoding::Wav),
            sample_rate: Some(24000),
        };

        match tts.synthesize(req).await {
            Ok(bytes) => Ok((bytes, "audio/wav".to_string())),
            Err(e) => {
                tracing::warn!(error = %e, "Cartesia TTS failed; attempting Google free TTS fallback");
                synthesize_google_free_tts(&state.http, &payload.text).await
            }
        }
    } else {
        synthesize_google_free_tts(&state.http, &payload.text).await
    };

    let (audio_bytes, mime_type) = audio_res.map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                code: "tts_error".into(),
                message: err,
            }),
        )
    })?;

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

    Ok(Json(VoiceSpeakResponse {
        audio_base64: b64,
        encoding: mime_type,
    }))
}

async fn synthesize_google_free_tts(
    http: &reqwest::Client,
    text: &str,
) -> Result<(Vec<u8>, String), String> {
    let text_clean = text.replace('\n', " ");
    let chunks = split_text_chunks(&text_clean, 200);
    let mut combined_bytes = Vec::new();

    for chunk in chunks {
        let encoded = url::form_urlencoded::byte_serialize(chunk.as_bytes()).collect::<String>();
        let url = format!("https://translate.google.com/translate_tts?ie=UTF-8&q={encoded}&tl=en&client=tw-ob");
        let res = http
            .get(&url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .send()
            .await
            .map_err(|e| format!("Google TTS transport error: {e}"))?;

        if !res.status().is_success() {
            return Err(format!("Google TTS status {}", res.status()));
        }

        let bytes = res
            .bytes()
            .await
            .map_err(|e| format!("Failed to read TTS bytes: {e}"))?;
        combined_bytes.extend_from_slice(&bytes);
    }

    Ok((combined_bytes, "audio/mp3".to_string()))
}

fn split_text_chunks(text: &str, max_len: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if current.len() + word.len() + 1 > max_len {
            if !current.is_empty() {
                chunks.push(current.clone());
                current.clear();
            }
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}

pub async fn diagnostic(
    State(state): State<SharedState>,
    Extension(_principal): Extension<Principal>,
) -> Json<VoiceDiagnosticResponse> {
    let configured = state
        .cartesia_api_key
        .as_ref()
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);

    Json(VoiceDiagnosticResponse {
        cartesia_configured: configured,
        stt_model: state.cartesia_stt_model.clone(),
        tts_model: state.cartesia_tts_model.clone(),
        tts_voice_id: state.cartesia_tts_voice_id.clone(),
        status: if configured {
            "ready".into()
        } else {
            "unconfigured".into()
        },
    })
}
