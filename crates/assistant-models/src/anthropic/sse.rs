//! Server-sent-event decoding for the Messages API stream.
//!
//! The decoder is a pure state machine over bytes: it is handed whatever
//! arrived from the socket and pushes provider-neutral [`StreamChunk`]s onto a
//! queue. Nothing here awaits, opens a connection or knows what HTTP is, which
//! is what makes the streaming behaviour testable without a network.
//!
//! The one piece of real state is tool-call assembly. A tool call arrives as a
//! `content_block_start` naming it, then a run of `input_json_delta` fragments
//! that are only valid JSON once concatenated, then a `content_block_stop`. The
//! call is emitted at the stop, never before -- a half-parsed argument object
//! must never reach the permission engine.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use crate::{ModelError, StreamChunk, Usage};

use super::wire;

/// One tool call being assembled across deltas.
struct PartialToolCall {
    id: String,
    name: String,
    json: String,
}

pub(super) type Queue = VecDeque<Result<StreamChunk, ModelError>>;

#[derive(Default)]
pub(super) struct SseDecoder {
    /// Bytes not yet forming a complete line. Kept as bytes, not as a `String`,
    /// because a multi-byte character can be split across two socket reads.
    buffer: Vec<u8>,
    blocks: HashMap<u64, PartialToolCall>,
    usage: Usage,
    /// Set once `message_stop` has been seen, so a truncated stream can be told
    /// apart from a complete one.
    complete: bool,
}

impl SseDecoder {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Feeds one socket read into the decoder.
    pub(super) fn push(&mut self, bytes: &[u8], out: &mut Queue) {
        self.buffer.extend_from_slice(bytes);

        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line);
            self.line(line.trim_end_matches(['\r', '\n']), out);
        }
    }

    /// Called when the socket ends.
    ///
    /// A stream that stopped without `message_stop` is reported as a transport
    /// failure rather than silently completed: the alternative is presenting a
    /// truncated answer as a finished one.
    pub(super) fn finish(&mut self, out: &mut Queue) {
        if !self.complete {
            out.push_back(Err(ModelError::Unavailable(
                "provider stream ended before the message was complete".into(),
            )));
        }
    }

    fn line(&mut self, line: &str, out: &mut Queue) {
        // `event:` lines are redundant -- every payload repeats its type in the
        // JSON -- and `:` lines are comments.
        let Some(payload) = line.strip_prefix("data:") else {
            return;
        };
        let payload = payload.trim();
        if payload.is_empty() {
            return;
        }

        let event: Value = match serde_json::from_str(payload) {
            Ok(event) => event,
            Err(error) => {
                out.push_back(Err(ModelError::MalformedResponse(format!(
                    "stream event was not valid JSON: {error}"
                ))));
                return;
            }
        };

        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.usage.input_tokens = wire::parse_usage(
                    event
                        .get("message")
                        .and_then(|message| message.get("usage")),
                )
                .input_tokens;
            }

            Some("content_block_start") => self.block_start(&event),
            Some("content_block_delta") => self.block_delta(&event, out),
            Some("content_block_stop") => self.block_stop(&event, out),

            Some("message_delta") => {
                let delta = event.get("delta");
                let output_tokens = wire::parse_usage(event.get("usage")).output_tokens;
                if output_tokens > 0 {
                    self.usage.output_tokens = output_tokens;
                }

                if delta
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                    == Some("refusal")
                {
                    out.push_back(Err(ModelError::Refused {
                        category: wire::stop_category(
                            delta.and_then(|delta| delta.get("stop_details")),
                        ),
                    }));
                }
            }

            Some("message_stop") => {
                self.complete = true;
                out.push_back(Ok(StreamChunk::Done(self.usage)));
            }

            // An error can arrive mid-stream, after some text has already been
            // delivered. It is reported as-is; the orchestrator decides what a
            // partially-answered turn means.
            Some("error") => out.push_back(Err(error_event(&event))),

            // `ping` and anything added later.
            _ => {}
        }
    }

    fn block_start(&mut self, event: &Value) {
        let Some(index) = index_of(event) else { return };
        let Some(block) = event.get("content_block") else {
            return;
        };
        if block.get("type").and_then(Value::as_str) != Some("tool_use") {
            return;
        }

        let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
        let name = block
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();

        self.blocks.insert(
            index,
            PartialToolCall {
                id: id.to_string(),
                name: name.to_string(),
                json: String::new(),
            },
        );
    }

    fn block_delta(&mut self, event: &Value, out: &mut Queue) {
        let Some(delta) = event.get("delta") else {
            return;
        };

        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => {
                if let Some(text) = delta.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    out.push_back(Ok(StreamChunk::Text(text.to_string())));
                }
            }
            Some("input_json_delta") => {
                let Some(index) = index_of(event) else { return };
                if let Some(partial) = delta.get("partial_json").and_then(Value::as_str)
                    && let Some(block) = self.blocks.get_mut(&index)
                {
                    block.json.push_str(partial);
                }
            }
            // Thinking and signature deltas are not part of the answer this
            // application streams to a user.
            _ => {}
        }
    }

    fn block_stop(&mut self, event: &Value, out: &mut Queue) {
        let Some(index) = index_of(event) else { return };
        let Some(block) = self.blocks.remove(&index) else {
            return;
        };

        // A tool call with no arguments streams no `input_json_delta` at all.
        let json = if block.json.trim().is_empty() {
            "{}"
        } else {
            block.json.as_str()
        };

        match serde_json::from_str::<Value>(json) {
            Ok(arguments) => out.push_back(Ok(StreamChunk::ToolCall(
                // Anthropic asks for nothing back on the next round, so there is
                // no provider state to carry.
                assistant_tools::ToolCall::new(block.id, block.name, arguments),
            ))),
            Err(error) => out.push_back(Err(ModelError::MalformedResponse(format!(
                "streamed tool arguments were not valid JSON: {error}"
            )))),
        }
    }
}

fn index_of(event: &Value) -> Option<u64> {
    event.get("index").and_then(Value::as_u64)
}

/// Maps an in-stream `error` event onto the structured error type.
fn error_event(event: &Value) -> ModelError {
    let error = event.get("error");
    let kind = error
        .and_then(|error| error.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let detail = wire::truncate(
        error
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("provider reported an error mid-stream"),
        300,
    );

    match kind {
        "authentication_error" | "permission_error" => ModelError::AuthFailed,
        "rate_limit_error" => ModelError::RateLimited { retry_after: None },
        "invalid_request_error" | "request_too_large" => ModelError::InvalidRequest(detail),
        "overloaded_error" | "api_error" => ModelError::Unavailable(detail),
        "timeout_error" => ModelError::Timeout,
        _ => ModelError::Rejected(detail),
    }
}
