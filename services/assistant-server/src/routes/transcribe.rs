//! OpenAI Audio Transcription endpoint.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Json,
};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};

use crate::state::SharedState;
use assistant_protocol::ApiError;

#[derive(Debug, Serialize, Deserialize)]
pub struct TranscribeResponse {
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIWhisperResponse {
    #[serde(default)]
    text: String,
    #[serde(default)]
    error: Option<OpenAIErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct OpenAIErrorDetail {
    message: Option<String>,
}

pub async fn transcribe(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<TranscribeResponse>, (StatusCode, Json<ApiError>)> {
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "empty_audio".into(),
                message: "Audio payload cannot be empty.".into(),
            }),
        ));
    }

    let api_key = state.openai_api_key.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                code: "openai_not_configured".into(),
                message: "OPENAI_API_KEY is not configured on the server.".into(),
            }),
        )
    })?;

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/webm");

    // Determine appropriate file extension
    let file_name = if content_type.contains("wav") {
        "audio.wav"
    } else if content_type.contains("m4a") || content_type.contains("mp4") {
        "audio.m4a"
    } else if content_type.contains("ogg") {
        "audio.ogg"
    } else {
        "audio.webm"
    };

    let audio_part = Part::bytes(body.to_vec())
        .file_name(file_name)
        .mime_str(content_type)
        .unwrap_or_else(|_| Part::bytes(body.to_vec()).file_name("audio.webm"));

    let model = if state.openai_transcription_model.is_empty()
        || state.openai_transcription_model == "gpt-live-transcribe"
    {
        // Fallback to whisper-1 if gpt-live-transcribe is set in .env
        // (whisper-1 is OpenAI's standard REST transcription model)
        "whisper-1"
    } else {
        &state.openai_transcription_model
    };

    let mut form = Form::new()
        .part("file", audio_part)
        .text("model", model.to_string())
        .text("response_format", "json");

    if let Some(lang) = &state.openai_transcription_language
        && !lang.trim().is_empty()
    {
        form = form.text("language", lang.clone());
    }

    let response = state
        .http
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to contact OpenAI transcription API");
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiError {
                    code: "transcription_transport_error".into(),
                    message: "Failed to reach transcription provider.".into(),
                }),
            )
        })?;

    let status = response.status();
    let text = response.text().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiError {
                code: "transcription_read_error".into(),
                message: format!("Failed to read transcription response: {e}"),
            }),
        )
    })?;

    if !status.is_success() {
        // The status is the diagnostic. The body is a provider response we do
        // not control and cannot vet, so it stays out of the log rather than
        // risking quota, account or prompt detail landing in it.
        tracing::error!(status = %status, "OpenAI transcription error");
        let parsed: Result<OpenAIWhisperResponse, _> = serde_json::from_str(&text);
        let detail = parsed
            .ok()
            .and_then(|p| p.error)
            .and_then(|e| e.message)
            .unwrap_or_else(|| "Transcription failed".into());

        return Err((
            StatusCode::BAD_GATEWAY,
            Json(ApiError {
                code: "transcription_error".into(),
                message: detail,
            }),
        ));
    }

    let parsed: OpenAIWhisperResponse = serde_json::from_str(&text).map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            Json(ApiError {
                code: "transcription_parse_error".into(),
                message: format!("Could not parse transcription output: {e}"),
            }),
        )
    })?;

    Ok(Json(TranscribeResponse {
        text: parsed.text.trim().to_string(),
    }))
}
