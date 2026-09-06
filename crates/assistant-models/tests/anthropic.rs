//! Anthropic provider tests.
//!
//! Every one of these runs against a local socket speaking canned HTTP, so the
//! suite needs no API key, no network and no account, and `cargo test
//! --workspace` stays runnable by anyone. What is under test is exactly the
//! code that would break against the real API: request construction, SSE
//! decoding, tool-call assembly and the status-to-error mapping.
//!
//! The fake server is deliberately dumb -- it replays bytes -- because a
//! smarter one would start to encode this build's assumptions about the API and
//! then agree with them.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use assistant_models::{
    Capability, GenerateRequest, Message, ModelError, ModelId, ModelProvider, StreamChunk,
    anthropic::{AnthropicConfig, AnthropicModelProvider},
};
use assistant_tools::{RiskLevel, ToolCall, ToolSpec};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEST_KEY: &str = "sk-ant-test-not-a-real-key";

/// What the fake server should do with a connection.
#[derive(Clone)]
enum Reply {
    /// A complete HTTP response, written at once.
    Whole { status: u16, body: String },
    /// A chunked SSE response, written one event per TCP write.
    Sse(Vec<String>),
    /// Accept the connection and never answer.
    Silence,
}

/// A local HTTP server that answers a fixed script of replies.
struct FakeApi {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FakeApi {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bound");
        let addr = listener.local_addr().expect("addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();

        tokio::spawn(async move {
            for reply in replies {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let request = read_request(&mut socket).await;
                recorded.lock().expect("not poisoned").push(request);

                match reply {
                    Reply::Whole { status, body } => {
                        let response = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nretry-after: 7\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.flush().await;
                    }
                    Reply::Sse(events) => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
                        let _ = socket.write_all(head.as_bytes()).await;
                        for event in events {
                            let frame = format!("{:x}\r\n{event}\r\n", event.len());
                            let _ = socket.write_all(frame.as_bytes()).await;
                            let _ = socket.flush().await;
                            // Separate writes, so a decoder that only works
                            // when the whole body arrives at once fails here.
                            tokio::time::sleep(Duration::from_millis(2)).await;
                        }
                        let _ = socket.write_all(b"0\r\n\r\n").await;
                        let _ = socket.flush().await;
                    }
                    Reply::Silence => {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                    }
                }
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            requests,
        }
    }

    /// The JSON body of the nth request the provider sent.
    fn body(&self, index: usize) -> Value {
        let raw = self.requests.lock().expect("not poisoned")[index].clone();
        let (_, body) = raw.split_once("\r\n\r\n").expect("request had a body");
        serde_json::from_str(body).expect("body was valid JSON")
    }

    fn raw(&self, index: usize) -> String {
        self.requests.lock().expect("not poisoned")[index].clone()
    }

    fn count(&self) -> usize {
        self.requests.lock().expect("not poisoned").len()
    }
}

/// Reads one HTTP request, honouring `content-length`.
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    loop {
        let read = match socket.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        buffer.extend_from_slice(&chunk[..read]);

        let text = String::from_utf8_lossy(&buffer).to_string();
        let Some(header_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let length: usize = text[..header_end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .unwrap_or(0);

        if buffer.len() >= header_end + 4 + length {
            break;
        }
    }

    String::from_utf8_lossy(&buffer).to_string()
}

fn provider(
    api: &FakeApi,
    tune: impl FnOnce(AnthropicConfig) -> AnthropicConfig,
) -> AnthropicModelProvider {
    let config = tune(
        AnthropicConfig::new(TEST_KEY)
            .with_base_url(&api.base_url)
            .with_max_transport_retries(0),
    );
    AnthropicModelProvider::new(config).expect("provider built")
}

fn request(messages: Vec<Message>) -> GenerateRequest {
    GenerateRequest {
        model: ModelId("ignored-the-config-decides".into()),
        system_prompt: Some("You are the assistant.".into()),
        messages,
        tools: Vec::new(),
        max_output_tokens: None,
        temperature: None,
    }
}

fn message_body(content: Value) -> String {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-opus-5",
        "content": content,
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 11, "output_tokens": 4}
    })
    .to_string()
}

fn sse(events: &[Value]) -> Reply {
    Reply::Sse(
        events
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {event}\n\n",
                    event["type"].as_str().unwrap()
                )
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_request_carries_the_documented_headers_and_never_an_authorization_header() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([{"type": "text", "text": "hi"}])),
    }])
    .await;

    provider(&api, |config| config)
        .generate(request(vec![Message::user("hello")]))
        .await
        .expect("answered");

    let raw = api.raw(0).to_lowercase();
    assert!(raw.contains("x-api-key: "), "missing api key header");
    assert!(raw.contains("anthropic-version: 2023-06-01"));
    assert!(
        !raw.contains("authorization:"),
        "the provider must not send an Authorization header"
    );
}

#[tokio::test]
async fn system_instructions_are_hoisted_out_of_the_message_list() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([{"type": "text", "text": "ok"}])),
    }])
    .await;

    provider(&api, |config| config)
        .generate(request(vec![
            Message::system("Relevant context: none."),
            Message::user("hello"),
        ]))
        .await
        .expect("answered");

    let body = api.body(0);
    let system = body["system"].as_str().expect("system field present");
    assert!(system.contains("You are the assistant."));
    assert!(system.contains("Relevant context: none."));

    let messages = body["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 1, "system leaked into the message list");
    assert_eq!(messages[0]["role"], "user");
}

#[tokio::test]
async fn tool_specs_become_declarations_without_their_risk_or_scopes() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([{"type": "text", "text": "ok"}])),
    }])
    .await;

    let mut generate = request(vec![Message::user("send it")]);
    generate.tools = vec![ToolSpec {
        name: "gmail.send".into(),
        description: "Send an email".into(),
        input_schema: json!({"type": "object", "properties": {"to": {"type": "string"}}}),
        output_schema: json!({"type": "object"}),
        risk: RiskLevel::Red,
        required_scopes: vec!["gmail.send".into()],
        timeout_ms: 5_000,
    }];

    provider(&api, |config| config)
        .generate(generate)
        .await
        .expect("answered");

    let body = api.body(0);
    let tool = &body["tools"][0];
    assert_eq!(tool["name"], "gmail.send");
    assert_eq!(tool["input_schema"]["type"], "object");

    let serialised = body["tools"].to_string();
    for leaked in ["red", "required_scopes", "timeout_ms"] {
        assert!(
            !serialised.contains(leaked),
            "server-side policy leaked to the model: {leaked}"
        );
    }
}

#[tokio::test]
async fn tool_results_become_one_user_message_of_tool_result_blocks() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([{"type": "text", "text": "done"}])),
    }])
    .await;

    provider(&api, |config| config)
        .generate(request(vec![
            Message::user("what is on my list"),
            Message::assistant_tool_calls(
                "Checking.",
                vec![
                    ToolCall {
                        id: "toolu_a".into(),
                        name: "notes.read".into(),
                        arguments: json!({"limit": 3}),
                    },
                    ToolCall {
                        id: "toolu_b".into(),
                        name: "tasks.read".into(),
                        arguments: json!({}),
                    },
                ],
            ),
            Message::tool_result("toolu_a", r#"{"notes":[]}"#),
            Message::tool_result("toolu_b", r#"{"tasks":[]}"#),
        ]))
        .await
        .expect("answered");

    let body = api.body(0);
    let messages = body["messages"].as_array().expect("messages");

    assert_eq!(messages.len(), 3, "roles did not alternate: {messages:?}");
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][0]["type"], "text");
    assert_eq!(messages[1]["content"][1]["type"], "tool_use");
    assert_eq!(messages[1]["content"][1]["id"], "toolu_a");

    // Both results in one user message, which is what the API requires.
    assert_eq!(messages[2]["role"], "user");
    let results = messages[2]["content"].as_array().expect("blocks");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "toolu_a");
    assert_eq!(results[1]["tool_use_id"], "toolu_b");
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_normal_response_is_parsed_into_text_and_usage() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([
            {"type": "thinking", "thinking": ""},
            {"type": "text", "text": "Hello, Alex."}
        ])),
    }])
    .await;

    let response = provider(&api, |config| config)
        .generate(request(vec![Message::user("hi")]))
        .await
        .expect("answered");

    assert_eq!(response.text.as_deref(), Some("Hello, Alex."));
    assert!(response.tool_calls.is_empty());
    assert_eq!(response.usage.input_tokens, 11);
    assert_eq!(response.usage.output_tokens, 4);
}

#[tokio::test]
async fn a_tool_use_block_becomes_a_tool_call() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([
            {"type": "text", "text": "Looking."},
            {"type": "tool_use", "id": "toolu_1", "name": "notes.read", "input": {"limit": 5}}
        ])),
    }])
    .await;

    let response = provider(&api, |config| config)
        .generate(request(vec![Message::user("read my notes")]))
        .await
        .expect("answered");

    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].id, "toolu_1");
    assert_eq!(response.tool_calls[0].name, "notes.read");
    assert_eq!(response.tool_calls[0].arguments["limit"], 5);
}

#[tokio::test]
async fn a_response_that_is_not_json_is_a_malformed_response_not_a_crash() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: "<html>gateway</html>".into(),
    }])
    .await;

    let error = provider(&api, |config| config)
        .generate(request(vec![Message::user("hi")]))
        .await
        .expect_err("rejected");

    assert_eq!(error.code(), "provider_error");
    assert!(matches!(error, ModelError::MalformedResponse(_)));
}

#[tokio::test]
async fn a_refusal_is_reported_as_a_refusal_not_as_an_empty_answer() {
    let body = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "content": [],
        "stop_reason": "refusal",
        "stop_details": {"type": "refusal", "category": "cyber"},
        "usage": {"input_tokens": 5, "output_tokens": 0}
    })
    .to_string();

    let api = FakeApi::start(vec![Reply::Whole { status: 200, body }]).await;

    let error = provider(&api, |config| config)
        .generate(request(vec![Message::user("hi")]))
        .await
        .expect_err("refused");

    assert_eq!(error.code(), "provider_refused");
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_authentication_failure_is_structured_and_says_nothing_about_the_key() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 401,
        body: json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}})
            .to_string(),
    }])
    .await;

    let error = provider(&api, |config| config)
        .generate(request(vec![Message::user("hi")]))
        .await
        .expect_err("rejected");

    assert_eq!(error.code(), "provider_auth_failed");
    assert!(!error.to_string().contains("x-api-key"));
    assert!(!error.user_message().contains("api"));
}

#[tokio::test]
async fn a_rate_limited_request_never_opens_a_stream() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 429,
        body:
            json!({"type": "error", "error": {"type": "rate_limit_error", "message": "slow down"}})
                .to_string(),
    }])
    .await;

    let error = match provider(&api, |config| config)
        .stream(request(vec![Message::user("hi")]))
        .await
    {
        Ok(_) => panic!("a rate-limited request must not open a stream"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "provider_rate_limited");
}

#[tokio::test]
async fn a_server_error_is_unavailable_and_a_bad_request_is_not() {
    for (status, expected) in [
        (500u16, "provider_unavailable"),
        (400, "provider_invalid_request"),
    ] {
        let api = FakeApi::start(vec![Reply::Whole {
            status,
            body: json!({"type": "error", "error": {"message": "boom"}}).to_string(),
        }])
        .await;

        let error = provider(&api, |config| config)
            .generate(request(vec![Message::user("hi")]))
            .await
            .expect_err("rejected");

        assert_eq!(error.code(), expected, "status {status}");
    }
}

#[tokio::test]
async fn a_provider_that_never_answers_times_out() {
    let api = FakeApi::start(vec![Reply::Silence]).await;

    let error = provider(&api, |config| {
        config.with_timeout(Duration::from_millis(250))
    })
    .generate(request(vec![Message::user("hi")]))
    .await
    .expect_err("timed out");

    assert_eq!(error.code(), "provider_timeout");
}

#[tokio::test]
async fn a_transport_failure_is_retried_once_but_never_with_tool_results_attached() {
    let overloaded = || Reply::Whole {
        status: 503,
        body: json!({"type": "error", "error": {"message": "unavailable"}}).to_string(),
    };

    // Without tool results: one retry, so two requests reach the server.
    let api = FakeApi::start(vec![overloaded(), overloaded()]).await;
    let error = provider(&api, |config| config.with_max_transport_retries(1))
        .generate(request(vec![Message::user("hi")]))
        .await
        .expect_err("gave up");
    assert_eq!(error.code(), "provider_unavailable");
    assert_eq!(api.count(), 2, "the safe request was not retried");

    // With tool results: the turn has already run something, so the request is
    // sent exactly once.
    let api = FakeApi::start(vec![overloaded(), overloaded()]).await;
    provider(&api, |config| config.with_max_transport_retries(1))
        .generate(request(vec![
            Message::user("send it"),
            Message::assistant_tool_calls(
                "",
                vec![ToolCall {
                    id: "toolu_a".into(),
                    name: "gmail.send".into(),
                    arguments: json!({}),
                }],
            ),
            Message::tool_result("toolu_a", "{\"sent\":true}"),
        ]))
        .await
        .expect_err("gave up");
    assert_eq!(
        api.count(),
        1,
        "a request carrying tool results was retried"
    );
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

#[tokio::test]
async fn streamed_text_arrives_incrementally_and_reassembles_exactly() {
    let api = FakeApi::start(vec![sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 9}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Your name "}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "is "}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Alex."}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 6}}),
        json!({"type": "message_stop"}),
    ])])
    .await;

    let mut stream = provider(&api, |config| config)
        .stream(request(vec![Message::user("what is my name")]))
        .await
        .expect("stream opened");

    let mut text = String::new();
    let mut deltas = 0;
    let mut usage = None;

    while let Some(chunk) = stream.next().await {
        match chunk.expect("no error") {
            StreamChunk::Text(part) => {
                deltas += 1;
                text.push_str(&part);
            }
            StreamChunk::Done(seen) => usage = Some(seen),
            StreamChunk::ToolCall(_) => panic!("no tool call expected"),
        }
    }

    assert_eq!(text, "Your name is Alex.");
    assert_eq!(deltas, 3, "the stream was not incremental");
    let usage = usage.expect("a Done chunk terminated the stream");
    assert_eq!(usage.input_tokens, 9);
    assert_eq!(usage.output_tokens, 6);
}

#[tokio::test]
async fn a_streamed_tool_call_is_emitted_only_once_its_arguments_are_complete() {
    let api = FakeApi::start(vec![sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 4}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "One moment."}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "toolu_9", "name": "notes.read", "input": {}}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"limit\""}}),
        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": ": 5}"}}),
        json!({"type": "content_block_stop", "index": 1}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 12}}),
        json!({"type": "message_stop"}),
    ])])
    .await;

    let mut stream = provider(&api, |config| config)
        .stream(request(vec![Message::user("read my notes")]))
        .await
        .expect("stream opened");

    let mut order = Vec::new();
    let mut calls: Vec<ToolCall> = Vec::new();

    while let Some(chunk) = stream.next().await {
        match chunk.expect("no error") {
            StreamChunk::Text(_) => order.push("text"),
            StreamChunk::ToolCall(call) => {
                order.push("call");
                calls.push(call);
            }
            StreamChunk::Done(_) => order.push("done"),
        }
    }

    assert_eq!(order, ["text", "call", "done"]);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "notes.read");
    assert_eq!(
        calls[0].arguments,
        json!({"limit": 5}),
        "fragments did not reassemble"
    );
}

#[tokio::test]
async fn an_error_event_mid_stream_terminates_the_stream() {
    let api = FakeApi::start(vec![sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "partial"}}),
        json!({"type": "error", "error": {"type": "overloaded_error", "message": "overloaded"}}),
    ])])
    .await;

    let mut stream = provider(&api, |config| config)
        .stream(request(vec![Message::user("hi")]))
        .await
        .expect("stream opened");

    let first = stream.next().await.expect("a chunk").expect("no error");
    assert!(matches!(first, StreamChunk::Text(ref part) if part == "partial"));

    let error = stream
        .next()
        .await
        .expect("a second item")
        .expect_err("the error surfaced");
    assert_eq!(error.code(), "provider_unavailable");

    assert!(
        stream.next().await.is_none(),
        "the stream continued past an error"
    );
}

#[tokio::test]
async fn a_stream_that_stops_early_is_never_reported_as_a_finished_answer() {
    let api = FakeApi::start(vec![sse(&[
        json!({"type": "message_start", "message": {"usage": {"input_tokens": 1}}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "half an ans"}}),
    ])])
    .await;

    let mut stream = provider(&api, |config| config)
        .stream(request(vec![Message::user("hi")]))
        .await
        .expect("stream opened");

    let mut saw_done = false;
    let mut saw_error = false;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(StreamChunk::Done(_)) => saw_done = true,
            Err(_) => saw_error = true,
            Ok(_) => {}
        }
    }

    assert!(!saw_done, "a truncated stream claimed to be complete");
    assert!(saw_error, "a truncated stream failed silently");
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_configured_model_is_what_is_sent_not_the_requests_model_id() {
    let api = FakeApi::start(vec![Reply::Whole {
        status: 200,
        body: message_body(json!([{"type": "text", "text": "ok"}])),
    }])
    .await;

    provider(&api, |config| config.with_model("claude-haiku-4-5"))
        .generate(request(vec![Message::user("hi")]))
        .await
        .expect("answered");

    assert_eq!(api.body(0)["model"], "claude-haiku-4-5");
}

#[test]
fn the_api_key_is_not_in_the_debug_output() {
    let config = AnthropicConfig::new(TEST_KEY);
    let shown = format!("{config:?}");
    assert!(!shown.contains(TEST_KEY), "the key leaked: {shown}");
    assert!(shown.contains("<redacted>"));

    let provider = AnthropicModelProvider::new(AnthropicConfig::new(TEST_KEY)).expect("built");
    assert!(!format!("{provider:?}").contains(TEST_KEY));
}

#[test]
fn a_provider_without_a_key_refuses_to_be_built() {
    let error = AnthropicModelProvider::new(AnthropicConfig::new("  ")).expect_err("refused");
    assert_eq!(error.code(), "provider_auth_failed");
}

#[test]
fn the_provider_declares_the_capabilities_the_orchestrator_relies_on() {
    let provider = AnthropicModelProvider::new(AnthropicConfig::new(TEST_KEY)).expect("built");
    assert_eq!(provider.name(), "anthropic");
    for capability in [
        Capability::Generate,
        Capability::Stream,
        Capability::ToolUse,
    ] {
        assert!(provider.supports(capability), "missing {capability:?}");
    }
    assert!(!provider.supports(Capability::RealtimeAudio));
}
