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

use crate::state::SharedState;

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

pub async fn transcribe(
    State(state): State<SharedState>,
    Extension(_principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<VoiceTranscribeResponse>, (StatusCode, Json<ApiError>)> {
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "empty_audio".into(),
                message: "Audio payload cannot be empty.".into(),
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
    Extension(_principal): Extension<Principal>,
    Json(payload): Json<VoiceSpeakRequest>,
) -> Result<Json<VoiceSpeakResponse>, (StatusCode, Json<ApiError>)> {
    if payload.text.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "empty_text".into(),
                message: "Text string cannot be empty.".into(),
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

    let tts = CartesiaTtsProvider::new(
        api_key.to_string(),
        Some(state.cartesia_tts_model.clone()),
        Some(
            payload
                .voice_id
                .unwrap_or_else(|| state.cartesia_tts_voice_id.clone()),
        ),
    );

    let req = TtsRequest {
        text: payload.text,
        voice_id: None,
        model: None,
        encoding: Some(AudioEncoding::Wav),
        sample_rate: Some(24000),
    };

    let audio_bytes = tts.synthesize(req).await.map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                code: "tts_error".into(),
                message: err.to_string(),
            }),
        )
    })?;

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);

    Ok(Json(VoiceSpeakResponse {
        audio_base64: b64,
        encoding: "audio/wav".into(),
    }))
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
