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

    let api_key = state
        .openai_api_key
        .as_deref()
        .or(state.gemini_api_key.as_deref())
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError {
                    code: "openai_not_configured".into(),
                    message: "Neither OPENAI_API_KEY nor GEMINI_API_KEY is configured on the server.".into(),
                }),
            )
        })?;

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/webm");

    let is_gemini = state.gemini_api_key.is_some()
        || state
            .openai_base_url
            .as_deref()
            .map(|u| u.contains("generativelanguage.googleapis.com"))
            .unwrap_or(false);

    if is_gemini {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&body);
        let mime = if content_type.is_empty() { "audio/webm" } else { content_type };
        let data_url = format!("data:{mime};base64,{b64}");

        let gemini_model = if state.openai_transcription_model.starts_with("gemini") {
            &state.openai_transcription_model
        } else {
            "gemini-1.5-flash"
        };

        let payload = serde_json::json!({
            "model": gemini_model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Transcribe this audio recording verbatim. Output ONLY the raw spoken text without quotes, formatting, or commentary."
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": data_url
                            }
                        }
                    ]
                }
            ]
        });

        let base_url = state
            .openai_base_url
            .as_deref()
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta/openai");
        let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

        let response = state
            .http
            .post(&url)
            .bearer_auth(api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "failed to contact Gemini transcription API");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(ApiError {
                        code: "transcription_transport_error".into(),
                        message: "Failed to reach Gemini transcription API.".into(),
                    }),
                )
            })?;

        let status = response.status();
        let resp_json: serde_json::Value = response.json().await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ApiError {
                    code: "transcription_read_error".into(),
                    message: format!("Failed to parse Gemini transcription response: {e}"),
                }),
            )
        })?;

        if !status.is_success() {
            let msg = resp_json["error"]["message"]
                .as_str()
                .unwrap_or("Gemini transcription failed");
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiError {
                    code: "transcription_error".into(),
                    message: msg.to_string(),
                }),
            ));
        }

        let transcribed_text = resp_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        return Ok(Json(TranscribeResponse {
            text: transcribed_text,
        }));
    }

    // Standard OpenAI Whisper multipart upload path
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

    let base_url = state
        .openai_base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");
    let transcription_url = format!("{}/v1/audio/transcriptions", base_url.trim_end_matches('/'));

    let response = state
        .http
        .post(&transcription_url)
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "failed to contact transcription API");
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
