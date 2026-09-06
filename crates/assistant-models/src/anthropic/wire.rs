//! Translation between this workspace's provider-neutral types and the
//! Anthropic Messages API wire format.
//!
//! Everything Anthropic-shaped stops here. `assistant-core` never sees a type
//! from this module, and this module never sees a `TurnEvent`, a `Principal` or
//! a `RiskLevel`.
//!
//! Two translations are load-bearing:
//!
//! * **System instructions.** The API has no `system` role inside `messages`,
//!   so every [`Role::System`] entry is folded into the top-level `system`
//!   field. That is also why a client cannot smuggle a system prompt in: the
//!   only thing reaching this function is what the server built.
//! * **Tool results.** The API models a tool result as a `tool_result` block in
//!   a *user* message, and rejects consecutive messages with the same role, so
//!   a run of [`Role::Tool`] messages coalesces into one user message.

use serde::Serialize;
use serde_json::Value;

use crate::{GenerateRequest, Message, ModelError, Role, Usage};
use assistant_tools::{ToolCall, ToolSpec};

use super::config::{AnthropicConfig, Effort};

#[derive(Debug, Serialize)]
pub(super) struct WireRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
    #[serde(skip_serializing_if = "is_false")]
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct OutputConfig {
    pub effort: Effort,
}

#[derive(Debug, Serialize)]
pub(super) struct WireMessage {
    pub role: &'static str,
    pub content: Vec<Block>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum Block {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
pub(super) struct WireTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Builds the request body.
///
/// `stream` is a parameter rather than a config read so one configuration can
/// serve both entry points.
pub(super) fn build_request(
    config: &AnthropicConfig,
    request: &GenerateRequest,
    stream: bool,
) -> WireRequest {
    let mut system: Vec<String> = Vec::new();
    if let Some(prompt) = &request.system_prompt
        && !prompt.trim().is_empty()
    {
        system.push(prompt.clone());
    }

    let mut messages: Vec<WireMessage> = Vec::new();

    for message in &request.messages {
        match message.role {
            Role::System => {
                if !message.content.trim().is_empty() {
                    system.push(message.content.clone());
                }
            }
            Role::User => push_blocks(&mut messages, "user", text_block(&message.content)),
            Role::Assistant => push_blocks(&mut messages, "assistant", assistant_blocks(message)),
            Role::Tool => push_blocks(&mut messages, "user", tool_result_blocks(message)),
        }
    }

    // The API requires the first message to be from the user. History that
    // starts with an assistant turn means something upstream trimmed badly;
    // dropping the leading turns beats a 400 the user cannot act on.
    while matches!(messages.first(), Some(first) if first.role == "assistant") {
        messages.remove(0);
    }

    WireRequest {
        model: config.model.clone(),
        max_tokens: config.max_output_tokens,
        system: (!system.is_empty()).then(|| system.join("\n\n")),
        messages,
        tools: request.tools.iter().map(tool_declaration).collect(),
        temperature: config.temperature,
        output_config: config.effort.map(|effort| OutputConfig { effort }),
        stream,
    }
}

/// Appends blocks, merging into the previous message when the role repeats.
fn push_blocks(messages: &mut Vec<WireMessage>, role: &'static str, blocks: Vec<Block>) {
    if blocks.is_empty() {
        return;
    }
    match messages.last_mut() {
        Some(last) if last.role == role => last.content.extend(blocks),
        _ => messages.push(WireMessage {
            role,
            content: blocks,
        }),
    }
}

fn text_block(content: &str) -> Vec<Block> {
    // An empty text block is rejected by the API, and an empty turn carries no
    // information anyway.
    if content.trim().is_empty() {
        return Vec::new();
    }
    vec![Block::Text {
        text: content.to_string(),
    }]
}

fn assistant_blocks(message: &Message) -> Vec<Block> {
    let mut blocks = text_block(&message.content);
    blocks.extend(message.tool_calls.iter().map(|call| Block::ToolUse {
        id: call.id.clone(),
        name: call.name.clone(),
        input: call.arguments.clone(),
    }));
    blocks
}

fn tool_result_blocks(message: &Message) -> Vec<Block> {
    let Some(tool_use_id) = &message.tool_call_id else {
        // A tool message with no call to answer cannot be expressed on the
        // wire. Dropping it is right: inventing an id would attach the result
        // to some other call.
        return Vec::new();
    };
    vec![Block::ToolResult {
        tool_use_id: tool_use_id.clone(),
        content: message.content.clone(),
    }]
}

/// Generates a tool declaration from the registry's authoritative [`ToolSpec`].
///
/// Only the three fields the API understands cross over. `risk`,
/// `required_scopes` and `timeout_ms` deliberately do not: they are the
/// server's business, and telling the model about them would invite it to argue
/// about them. See ADR-0005.
fn tool_declaration(spec: &ToolSpec) -> WireTool {
    WireTool {
        name: spec.name.clone(),
        description: spec.description.clone(),
        input_schema: if spec.input_schema.is_object() {
            spec.input_schema.clone()
        } else {
            serde_json::json!({ "type": "object" })
        },
    }
}

/// What a non-streaming response yielded.
pub(super) struct ParsedMessage {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub stop_reason: Option<String>,
    pub stop_category: Option<String>,
}

/// Parses a complete `message` response.
///
/// Content blocks are read as JSON rather than through a typed enum on purpose:
/// the API gains block types over time (thinking blocks, server-tool results),
/// and a strict enum would turn a new block type into a hard failure on a
/// response that is otherwise perfectly usable.
pub(super) fn parse_message(body: &Value) -> Result<ParsedMessage, ModelError> {
    let blocks = body
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ModelError::MalformedResponse("response had no content array".into()))?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
            Some("tool_use") => tool_calls.push(parse_tool_use(block)?),
            // Thinking blocks and anything added later are not part of the
            // answer this application shows or replays.
            _ => {}
        }
    }

    Ok(ParsedMessage {
        text: (!text.is_empty()).then_some(text),
        tool_calls,
        usage: parse_usage(body.get("usage")),
        stop_reason: body
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        stop_category: stop_category(body.get("stop_details")),
    })
}

pub(super) fn stop_category(details: Option<&Value>) -> Option<String> {
    details
        .and_then(|details| details.get("category"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(super) fn parse_tool_use(block: &Value) -> Result<ToolCall, ModelError> {
    let id = block
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelError::MalformedResponse("tool_use block had no id".into()))?;
    let name = block
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ModelError::MalformedResponse("tool_use block had no name".into()))?;

    Ok(ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        // Arguments are handed to the core exactly as received. Validation
        // against the registry's schema happens there, against the
        // authoritative spec -- not here, against what the model claimed.
        arguments: block.get("input").cloned().unwrap_or(Value::Null),
    })
}

pub(super) fn parse_usage(usage: Option<&Value>) -> Usage {
    let field = |name: &str| {
        usage
            .and_then(|usage| usage.get(name))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
    };
    Usage {
        input_tokens: field("input_tokens"),
        output_tokens: field("output_tokens"),
    }
}

/// Reads `error.message` out of an error body, for the log only.
///
/// Truncated, because a provider error body is untrusted text of unbounded
/// length. It never reaches the user: [`ModelError::user_message`] does.
pub(super) fn error_detail(body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().to_string());

    truncate(&message, 300)
}

pub(super) fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect::<String>() + "..."
}
