//! Translation between this workspace's provider-neutral types and the
//! OpenAI Chat Completions API wire format.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{GenerateRequest, ModelError, Role, Usage};
use assistant_tools::{ToolCall, ToolSpec};

use super::config::OpenAIConfig;

#[derive(Debug, Serialize)]
pub(super) struct WireRequest {
    pub model: String,
    pub messages: Vec<WireMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<WireTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
pub(super) struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct WireMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<WireToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireToolCall {
    pub id: String,
    pub r#type: String,
    pub function: WireFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WireFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WireTool {
    pub r#type: &'static str,
    pub function: WireFunction,
}

#[derive(Debug, Serialize)]
pub(super) struct WireFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

pub(super) fn build_request(
    config: &OpenAIConfig,
    request: &GenerateRequest,
    stream: bool,
) -> WireRequest {
    let mut messages = Vec::new();

    if let Some(system) = &request.system_prompt
        && !system.trim().is_empty()
    {
        messages.push(WireMessage {
            role: "system".into(),
            content: Some(system.clone()),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    for msg in &request.messages {
        match msg.role {
            Role::System => {
                messages.push(WireMessage {
                    role: "system".into(),
                    content: Some(msg.content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            Role::User => {
                messages.push(WireMessage {
                    role: "user".into(),
                    content: Some(msg.content.clone()),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }
            Role::Assistant => {
                let tool_calls = if msg.tool_calls.is_empty() {
                    None
                } else {
                    Some(
                        msg.tool_calls
                            .iter()
                            .map(|tc| WireToolCall {
                                id: tc.id.clone(),
                                r#type: "function".into(),
                                function: WireFunctionCall {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.to_string(),
                                },
                            })
                            .collect(),
                    )
                };

                let content = if msg.content.is_empty() && tool_calls.is_some() {
                    None
                } else {
                    Some(msg.content.clone())
                };

                messages.push(WireMessage {
                    role: "assistant".into(),
                    content,
                    tool_calls,
                    tool_call_id: None,
                });
            }
            Role::Tool => {
                messages.push(WireMessage {
                    role: "tool".into(),
                    content: Some(msg.content.clone()),
                    tool_calls: None,
                    tool_call_id: msg.tool_call_id.clone(),
                });
            }
        }
    }

    let tools = request
        .tools
        .iter()
        .map(|tool: &ToolSpec| WireTool {
            r#type: "function",
            function: WireFunction {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            },
        })
        .collect();

    WireRequest {
        model: if request.model.0.is_empty() {
            config.model.clone()
        } else {
            request.model.0.clone()
        },
        messages,
        tools,
        max_tokens: request.max_output_tokens.or(Some(config.max_output_tokens)),
        temperature: request.temperature.or(config.temperature),
        stream,
        stream_options: if stream {
            Some(StreamOptions {
                include_usage: true,
            })
        } else {
            None
        },
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatCompletionResponse {
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<WireUsage>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatChoice {
    pub message: WireResponseMessage,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireResponseMessage {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<WireToolCall>>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WireUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

pub(super) fn parse_response(
    response: &Value,
) -> Result<(Option<String>, Vec<ToolCall>, Usage), ModelError> {
    let parsed: ChatCompletionResponse = serde_json::from_value(response.clone())
        .map_err(|e| ModelError::MalformedResponse(e.to_string()))?;

    let choice = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ModelError::MalformedResponse("no choices in response".into()))?;

    let tool_calls = choice
        .message
        .tool_calls
        .unwrap_or_default()
        .into_iter()
        .map(|tc| {
            let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null);
            ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments: args,
            }
        })
        .collect();

    let usage = parsed
        .usage
        .map(|u| Usage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        })
        .unwrap_or_default();

    Ok((choice.message.content, tool_calls, usage))
}
