//! Security and input-validation tests for the voice routes.
//!
//! These boot the real router and talk to it over real HTTP. Nothing about the
//! voice path is stubbed: the routes run their real auth layer and their real
//! validation, and the provider is simply unconfigured, which is the state
//! these tests care about.
//!
//! They exist because the voice routes previously accepted an unbounded body
//! and forwarded it straight to a paid provider.

use std::net::SocketAddr;

use assistant_core::EventBus;
use assistant_server::{app, config::Config, orchestration::Dependencies};

const TEST_TOKEN: &str = "integration-test-token";

fn config() -> Config {
    Config {
        bind_addr: "127.0.0.1:0".parse().expect("valid address"),
        database_url: None,
        dev_auth_token: TEST_TOKEN.to_string(),
        supabase_project_ref: None,
        allowed_origins: vec!["http://localhost:1420".to_string()],
        log_filter: "off".to_string(),
        max_tool_rounds: 4,
        openai_api_key: None,
        gemini_api_key: None,
        openai_base_url: None,
        openai_transcription_model: "whisper-1".to_string(),
        openai_transcription_language: None,
        model: "mock".to_string(),
        model_max_output_tokens: 256,
        model_timeout: std::time::Duration::from_secs(30),
        context_max_messages: 20,
        google_client_id: None,
        google_client_secret: None,
        google_redirect_uri: None,
        credential_encryption_key: None,
        document_storage_dir: std::env::temp_dir().join("assistant-voice-tests"),
        cartesia_api_key: None,
        cartesia_stt_model: "ink-whisper".to_string(),
        cartesia_tts_model: "sonic-3.6".to_string(),
        cartesia_tts_voice_id: "a0e99841-438c-4a64-b679-ae501e7d6091".to_string(),
    }
}

async fn spawn() -> SocketAddr {
    let cfg = config();
    let listener = tokio::net::TcpListener::bind(cfg.bind_addr)
        .await
        .expect("bound");
    let addr = listener.local_addr().expect("local addr");

    // These tests never reach the orchestrator; the defaults are enough to boot
    // the router so the voice routes can be exercised.
    let deps = Dependencies::default();

    let router = app(&cfg, None, EventBus::default(), deps);
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("server runs");
    });
    addr
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[tokio::test]
async fn voice_routes_reject_a_request_with_no_token() {
    let addr = spawn().await;

    for (method, path) in [
        ("POST", "/v1/voice/transcribe"),
        ("POST", "/v1/voice/speak"),
        ("GET", "/v1/voice/diagnostic"),
    ] {
        let url = format!("http://{addr}{path}");
        let req = match method {
            "POST" => client().post(&url).body("x"),
            _ => client().get(&url),
        };
        let res = req.send().await.expect("request sent");
        assert_eq!(
            res.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{path} answered an unauthenticated caller"
        );
    }
}

#[tokio::test]
async fn voice_routes_reject_a_forged_token() {
    let addr = spawn().await;
    let res = client()
        .post(format!("http://{addr}/v1/voice/speak"))
        .header("Authorization", "Bearer not-the-dev-token")
        .json(&serde_json::json!({ "text": "hello" }))
        .send()
        .await
        .expect("request sent");

    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn transcribe_rejects_an_oversized_body_before_calling_the_provider() {
    let addr = spawn().await;

    // Just over the handler's 10 MB ceiling, and below the transport backstop,
    // so the handler answers with its own coded error rather than the framework
    // dropping the connection. Previously any size was forwarded straight to a
    // paid provider.
    let body = vec![0u8; 10 * 1024 * 1024 + 1024];
    let res = client()
        .post(format!("http://{addr}/v1/voice/transcribe"))
        .header("Authorization", format!("Bearer {TEST_TOKEN}"))
        .header("Content-Type", "audio/wav")
        .body(body)
        .send()
        .await
        .expect("request sent");

    assert_eq!(res.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    let err: serde_json::Value = res.json().await.expect("error body");
    assert_eq!(err["code"], "audio_too_large");
}

#[tokio::test]
async fn transcribe_rejects_an_empty_body() {
    let addr = spawn().await;
    let res = client()
        .post(format!("http://{addr}/v1/voice/transcribe"))
        .header("Authorization", format!("Bearer {TEST_TOKEN}"))
        .header("Content-Type", "audio/wav")
        .body(Vec::<u8>::new())
        .send()
        .await
        .expect("request sent");

    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn speak_rejects_text_over_the_length_limit() {
    let addr = spawn().await;

    // Synthesis is billed by length, so this is a spend ceiling as much as a
    // validation rule.
    let text = "a".repeat(4_001);
    let res = client()
        .post(format!("http://{addr}/v1/voice/speak"))
        .header("Authorization", format!("Bearer {TEST_TOKEN}"))
        .json(&serde_json::json!({ "text": text }))
        .send()
        .await
        .expect("request sent");

    assert_eq!(res.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    let err: serde_json::Value = res.json().await.expect("error body");
    assert_eq!(err["code"], "text_too_long");
}

#[tokio::test]
async fn an_unconfigured_provider_says_so_rather_than_inventing_a_result() {
    let addr = spawn().await;

    // No Cartesia key in this config. The honest answer is that the service is
    // unavailable -- not a fabricated transcript, which is what the WebSocket
    // path used to return.
    let res = client()
        .post(format!("http://{addr}/v1/voice/transcribe"))
        .header("Authorization", format!("Bearer {TEST_TOKEN}"))
        .header("Content-Type", "audio/wav")
        .body(vec![1u8, 2, 3, 4])
        .send()
        .await
        .expect("request sent");

    assert_eq!(res.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    let err: serde_json::Value = res.json().await.expect("error body");
    assert_eq!(err["code"], "cartesia_not_configured");

    let message = err["message"].as_str().unwrap_or_default();
    assert!(
        !message.contains("sk_"),
        "error body leaked a key: {message}"
    );
}

#[tokio::test]
async fn the_diagnostic_never_returns_the_api_key() {
    let addr = spawn().await;
    let res = client()
        .get(format!("http://{addr}/v1/voice/diagnostic"))
        .header("Authorization", format!("Bearer {TEST_TOKEN}"))
        .send()
        .await
        .expect("request sent");

    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let body = res.text().await.expect("body");
    assert!(!body.contains("sk_"), "diagnostic leaked a key: {body}");
    assert!(
        !body.to_lowercase().contains("api_key"),
        "diagnostic exposed an api_key field: {body}"
    );
}

#[tokio::test]
async fn repeated_requests_are_rate_limited_per_principal() {
    let addr = spawn().await;

    // The limit is 30 per minute per bucket. Drive past it and the endpoint must
    // refuse rather than forwarding every one to a paid provider.
    let mut refused = None;
    for _ in 0..40 {
        let res = client()
            .post(format!("http://{addr}/v1/voice/speak"))
            .header("Authorization", format!("Bearer {TEST_TOKEN}"))
            .json(&serde_json::json!({ "text": "hello" }))
            .send()
            .await
            .expect("request sent");

        if res.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            refused = Some(res);
            break;
        }
    }

    let res = refused.expect("the endpoint never refused within 40 requests");
    let err: serde_json::Value = res.json().await.expect("error body");
    assert_eq!(err["code"], "rate_limited");
}
