//! One real request to the Anthropic API.
//!
//! `#[ignore]`, so `cargo test --workspace` compiles it and does not run it.
//! CI must never depend on an external AI API being up, and a test suite that
//! silently spends money every run is a test suite people stop running.
//!
//! Run it deliberately:
//!
//! ```powershell
//! $env:ANTHROPIC_API_KEY = "<key>"
//! cargo test -p assistant-models --test live_anthropic -- --ignored --nocapture
//! ```
//!
//! It makes exactly one request, with a trivial prompt and a tiny output cap.
//! It asserts things a mock could not fake -- that the socket streamed, and
//! that the model answered the prompt rather than replaying a script -- and it
//! prints no part of the credential.

use assistant_models::{
    GenerateRequest, Message, ModelId, ModelProvider, StreamChunk,
    anthropic::{AnthropicConfig, AnthropicModelProvider},
};
use futures::StreamExt;

#[tokio::test]
#[ignore = "makes a real, billable API request; run explicitly"]
async fn one_real_streamed_turn() {
    let _ = dotenvy::dotenv();

    let Some(key) = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
    else {
        // Skipped rather than failed: the same reason `--workspace` must not
        // need credentials applies to a developer running this by hand.
        eprintln!("skipped: ANTHROPIC_API_KEY is not set");
        return;
    };

    let config = AnthropicConfig::new(&key).with_max_output_tokens(64);
    let model = config.model.clone();
    let provider = AnthropicModelProvider::new(config).expect("provider built");

    let mut stream = provider
        .stream(GenerateRequest {
            model: ModelId(model.clone()),
            system_prompt: Some("Follow the user's formatting instruction exactly.".into()),
            messages: vec![Message::user(
                "Reply with exactly: connection test successful.",
            )],
            // No tools are offered, so no tool call can occur. That is the
            // assertion: a turn with no tools must not produce one.
            tools: Vec::new(),
            max_output_tokens: None,
            temperature: None,
        })
        .await
        .expect("the provider opened a stream");

    let mut text = String::new();
    let mut deltas = 0usize;
    let mut usage = None;

    while let Some(chunk) = stream.next().await {
        match chunk.expect("no provider error") {
            StreamChunk::Text(part) => {
                deltas += 1;
                text.push_str(&part);
            }
            StreamChunk::ToolCall(call) => {
                panic!(
                    "a turn offering no tools produced a tool call: {}",
                    call.name
                )
            }
            StreamChunk::Done(seen) => usage = Some(seen),
        }
    }

    let usage = usage.expect("the stream terminated with a Done chunk");

    // Printed so a human running this can see it really happened. The key is
    // not in scope here and nothing derived from it is printed.
    println!(
        "model={model} deltas={deltas} input_tokens={} output_tokens={} answer={:?}",
        usage.input_tokens, usage.output_tokens, text
    );

    assert!(!text.trim().is_empty(), "the model returned nothing");
    assert!(
        text.to_lowercase().contains("connection test successful"),
        "the model did not answer the prompt: {text:?}"
    );
    assert!(usage.input_tokens > 0, "no usage was reported");
    assert!(deltas >= 1, "nothing was streamed");
    assert!(
        !text.contains(&key),
        "the credential appeared in the response"
    );
}
