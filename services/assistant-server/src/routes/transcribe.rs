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

/// Only the success shape is modelled. The error shape used to be parsed so its
/// `message` could be forwarded to the client, which is exactly what ADR-0040
/// stops; an unparsed error body cannot leak.
#[derive(Debug, Deserialize)]
struct OpenAIWhisperResponse {
    #[serde(default)]
    text: String,
}

/// Maps an upstream transcription failure onto something a client may be told.
///
/// A provider's own error message names quotas, metrics, model ids and billing
/// URLs. That text was copied verbatim into `ApiError.message`, which the mobile
/// client renders, so a Gemini quota page appeared in red under the voice orb on
/// a physical device. Only the status crosses this boundary; the upstream text
/// stays in the `tracing` line above each call site. See ADR-0040.
fn upstream_error(status: StatusCode) -> (StatusCode, Json<ApiError>) {
    let (code, message) = if status == StatusCode::TOO_MANY_REQUESTS {
        (
            "transcription_rate_limited",
            "Transcription is rate limited right now. Try again in a moment.",
        )
    } else {
        ("transcription_error", "Transcription failed. Try again.")
    };

    (
        StatusCode::BAD_GATEWAY,
        Json(ApiError {
            code: code.into(),
            message: message.into(),
        }),
    )
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

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/webm");

    let is_custom_local = state
        .openai_base_url
        .as_deref()
        .map(|u| !u.contains("generativelanguage.googleapis.com"))
        .unwrap_or(false);

    let is_gemini = !is_custom_local
        && (state.gemini_api_key.is_some()
            || (state.openai_api_key.is_none() && state.openai_base_url.is_none())
            || state
                .openai_base_url
                .as_deref()
                .map(|u| u.contains("generativelanguage.googleapis.com"))
                .unwrap_or(false)
            || state.model.starts_with("gemini"));

    // The credential has to match the endpoint the request is about to go to.
    let api_key = if is_gemini {
        state.gemini_api_key.as_deref().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiError {
                    code: "transcription_not_configured".into(),
                    message: "Transcription is routed to Gemini but GEMINI_API_KEY is not configured on the server.".into(),
                }),
            )
        })?
    } else {
        match state.openai_api_key.as_deref() {
            Some(key) => key,
            None => {
                if is_custom_local {
                    "local-key"
                } else {
                    return Err((
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(ApiError {
                            code: "openai_not_configured".into(),
                            message: "OPENAI_API_KEY is not configured on the server.".into(),
                        }),
                    ));
                }
            }
        }
    };

    if is_gemini {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&body);
        let mime = if content_type.is_empty() {
            "audio/webm"
        } else {
            content_type
        };

        let primary_model = if state.openai_transcription_model.starts_with("gemini") {
            state.openai_transcription_model.as_str()
        } else if state.model.starts_with("gemini") {
            state.model.as_str()
        } else {
            "gemini-3.6-flash"
        };

        // Retry the configured model on a rate limit or a server spike, backing
        // off between attempts.
        //
        // This used to fall back onto `gemini-2.5-flash` and `gemini-2.0-flash`.
        // Both are now retired and answer `404 ... is no longer available`, and
        // 404 is not retryable -- so a transient 429 on the configured model was
        // reported to the user as "this model is no longer available", naming a
        // model the deployment had not asked for. Backing off on the model that
        // is actually configured keeps the reported cause the real one.
        const ATTEMPTS: usize = 3;
        // The last upstream status, not the last upstream message. See
        // `upstream_error`: the message is a diagnostic for the log, and the
        // status is the only part of it a client is told about.
        let mut last_status: Option<StatusCode> = None;

        for attempt in 0..ATTEMPTS {
            let model_name = &primary_model;
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(400 * attempt as u64)).await;
            }

            let payload = serde_json::json!({
                "contents": [
                    {
                        "parts": [
                            {
                                "text": "Transcribe this audio recording verbatim. Output ONLY the exact spoken English text. Do not add quotes, markdown formatting, or commentary."
                            },
                            {
                                "inlineData": {
                                    "mimeType": mime,
                                    "data": b64
                                }
                            }
                        ]
                    }
                ],
                "generationConfig": {
                    "temperature": 0.0
                }
            });

            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model_name}:generateContent?key={api_key}"
            );

            let response = match state.http.post(&url).json(&payload).send().await {
                Ok(res) => res,
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "Gemini transcription network transport attempt failed");
                    continue;
                }
            };

            let status = response.status();
            let resp_json: serde_json::Value = match response.json().await {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(attempt, error = %e, "Failed to parse Gemini response JSON");
                    continue;
                }
            };

            if !status.is_success() {
                let msg = resp_json["error"]["message"]
                    .as_str()
                    .unwrap_or("Gemini transcription failed");
                tracing::warn!(status = %status, attempt, model = %model_name, error_message = %msg, "Gemini STT spike / error");
                last_status = Some(status);

                // If 529 (Overloaded), 503 (Unavailable), or 429 (Rate Limit), retry with next model candidate
                if status == StatusCode::SERVICE_UNAVAILABLE
                    || status.as_u16() == 529
                    || status == StatusCode::TOO_MANY_REQUESTS
                {
                    continue;
                }

                // For fatal errors (e.g. invalid key), fail early
                return Err(upstream_error(status));
            }

            let transcribed_text = resp_json["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();

            return Ok(Json(TranscribeResponse {
                text: transcribed_text,
            }));
        }

        // Every attempt failed. A transport or parse failure leaves no status,
        // and is reported as a generic transcription failure rather than as a
        // rate limit it was not.
        return Err(upstream_error(
            last_status.unwrap_or(StatusCode::BAD_GATEWAY),
        ));
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
        // risking quota, account or prompt detail landing in it -- and, for the
        // same reason, out of the response. It used to be forwarded verbatim.
        tracing::error!(status = %status, "OpenAI transcription error");
        return Err(upstream_error(status));
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
