//! Server-sent-event decoding for the OpenAI Chat Completions stream.

use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

use crate::{ModelError, StreamChunk, Usage};
use assistant_tools::ToolCall;

pub(super) type Queue = VecDeque<Result<StreamChunk, ModelError>>;

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
    /// Provider state carried on the delta. Gemini attaches a
    /// `thought_signature` here and refuses the follow-up round without it.
    extra_content: Option<Value>,
}

#[derive(Default)]
pub(super) struct SseDecoder {
    buffer: Vec<u8>,
    tool_calls: HashMap<u64, PartialToolCall>,
    usage: Usage,
    complete: bool,
}

#[derive(Deserialize)]
struct ChunkPayload {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    #[serde(default)]
    delta: ChunkDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChunkToolCall>>,
}

#[derive(Deserialize)]
struct ChunkToolCall {
    #[serde(default)]
    index: u64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChunkFunction>,
    #[serde(default)]
    extra_content: Option<Value>,
}

#[derive(Deserialize)]
struct ChunkFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

impl SseDecoder {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn push(&mut self, bytes: &[u8], out: &mut Queue) {
        self.buffer.extend_from_slice(bytes);

        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line);
            self.line(line.trim_end_matches(['\r', '\n']), out);
        }
    }

    pub(super) fn finish(&mut self, out: &mut Queue) {
        if !self.buffer.is_empty() {
            let buffer = std::mem::take(&mut self.buffer);
            let remaining = String::from_utf8_lossy(&buffer);
            self.line(remaining.trim_end_matches(['\r', '\n']), out);
        }

        self.flush_tool_calls(out);

        if !self.complete {
            self.complete = true;
            out.push_back(Ok(StreamChunk::Done(self.usage)));
        }
    }

    fn line(&mut self, line: &str, out: &mut Queue) {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            return;
        }

        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let data = data.trim();

        if data == "[DONE]" {
            self.flush_tool_calls(out);
            self.complete = true;
            out.push_back(Ok(StreamChunk::Done(self.usage)));
            return;
        }

        let payload: ChunkPayload = match serde_json::from_str(data) {
            Ok(p) => p,
            Err(e) => {
                out.push_back(Err(ModelError::MalformedResponse(format!(
                    "invalid SSE chunk: {e}"
                ))));
                return;
            }
        };

        if let Some(u) = payload.usage {
            self.usage = Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            };
        }

        for choice in payload.choices {
            if let Some(text) = choice.delta.content
                && !text.is_empty()
            {
                out.push_back(Ok(StreamChunk::Text(text)));
            }

            if let Some(tool_calls) = choice.delta.tool_calls {
                for tc in tool_calls {
                    let entry = self.tool_calls.entry(tc.index).or_default();
                    if let Some(id) = tc.id {
                        entry.id = id;
                    }
                    if tc.extra_content.is_some() {
                        entry.extra_content = tc.extra_content;
                    }
                    if let Some(f) = tc.function {
                        if let Some(name) = f.name {
                            entry.name = name;
                        }
                        if let Some(args) = f.arguments {
                            entry.arguments.push_str(&args);
                        }
                    }
                }
            }

            if let Some(finish_reason) = choice.finish_reason
                && (finish_reason == "tool_calls" || finish_reason == "stop")
            {
                self.flush_tool_calls(out);
            }
        }
    }

    fn flush_tool_calls(&mut self, out: &mut Queue) {
        let mut keys: Vec<u64> = self.tool_calls.keys().copied().collect();
        keys.sort_unstable();

        for key in keys {
            if let Some(tc) = self.tool_calls.remove(&key)
                && !tc.name.is_empty()
            {
                let args: Value = serde_json::from_str(&tc.arguments).unwrap_or(Value::Null);
                out.push_back(Ok(StreamChunk::ToolCall(ToolCall {
                    id: tc.id,
                    name: tc.name,
                    arguments: args,
                    provider_metadata: tc.extra_content,
                })));
            }
        }
    }
}
