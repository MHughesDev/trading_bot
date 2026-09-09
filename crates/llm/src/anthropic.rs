//! Anthropic Messages adapter (`POST /v1/messages`).
//!
//! Wire quirks handled here:
//! - auth is `x-api-key` + `anthropic-version`, not a bearer header
//! - `system` is a top-level field, not a message
//! - tool results are `tool_result` blocks inside a **user** message; roles
//!   must alternate, so consecutive `ToolResult`s merge into one user message
//! - responses interleave `text` and `tool_use` content blocks

use serde_json::{json, Map, Value};

use crate::error::{classify_status, truncate, LlmError};
use crate::types::{ChatRequest, ChatResponse, Message, ModelInfo, StopReason, ToolCall, Usage};

const ANTHROPIC_VERSION: &str = "2023-06-01";

fn build_messages(req: &ChatRequest) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for msg in &req.messages {
        match msg {
            Message::User { content } => out.push(json!({ "role": "user", "content": content })),
            Message::Assistant {
                content,
                tool_calls,
            } => {
                let mut blocks: Vec<Value> = Vec::new();
                if let Some(text) = content {
                    if !text.is_empty() {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                }
                for call in tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    }));
                }
                out.push(json!({ "role": "assistant", "content": blocks }));
            }
            Message::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                    "is_error": is_error,
                });
                // Merge consecutive tool results into one user message so
                // roles keep alternating.
                let merged = match out.last_mut() {
                    Some(last)
                        if last.get("role").and_then(|r| r.as_str()) == Some("user")
                            && last
                                .get("content")
                                .and_then(|c| c.as_array())
                                .and_then(|a| a.last())
                                .and_then(|b| b.get("type"))
                                .and_then(|t| t.as_str())
                                == Some("tool_result") =>
                    {
                        last.get_mut("content")
                            .and_then(|c| c.as_array_mut())
                            .map(|arr| arr.push(block.clone()))
                            .is_some()
                    }
                    _ => false,
                };
                if !merged {
                    out.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }
    }
    out
}

fn build_body(req: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("max_tokens".into(), json!(req.max_tokens));
    body.insert("messages".into(), Value::Array(build_messages(req)));
    if let Some(system) = &req.system {
        body.insert("system".into(), json!(system));
    }
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    Value::Object(body)
}

fn parse_response(body: Value) -> Result<ChatResponse, LlmError> {
    let blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| LlmError::Deserialize("no content blocks in response".into()))?;

    let mut text_parts: Vec<&str> = Vec::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(t);
                }
            }
            Some("tool_use") => tool_calls.push(ToolCall {
                id: block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                arguments: block.get("input").cloned().unwrap_or(json!({})),
            }),
            _ => {}
        }
    }

    let stop_reason = match body.get("stop_reason").and_then(|v| v.as_str()) {
        Some("end_turn") | Some("stop_sequence") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some(other) => StopReason::Other(other.to_string()),
        None => StopReason::Other("missing".into()),
    };

    let usage = Usage {
        input_tokens: body
            .pointer("/usage/input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: body
            .pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    };

    Ok(ChatResponse {
        content: if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        },
        tool_calls,
        stop_reason,
        usage,
    })
}

pub async fn chat(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    req: &ChatRequest,
) -> Result<ChatResponse, LlmError> {
    let resp = http
        .post(format!("{base_url}/v1/messages"))
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&build_body(req))
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let text = resp
        .text()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;

    if status == 400 && text.to_lowercase().contains("prompt is too long") {
        return Err(LlmError::ContextTooLarge(truncate(&text)));
    }
    if !(200..300).contains(&status) {
        return Err(classify_status(status, &text, retry_after));
    }

    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| LlmError::Deserialize(e.to_string()))?;
    parse_response(parsed)
}

pub async fn list_models(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, LlmError> {
    let resp = http
        .get(format!("{base_url}/v1/models"))
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(classify_status(status, &text, None));
    }
    let body: Value =
        serde_json::from_str(&text).map_err(|e| LlmError::Deserialize(e.to_string()))?;
    Ok(body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(|v| v.as_str())?;
                    Some(ModelInfo {
                        id: id.to_string(),
                        display_name: m
                            .get("display_name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_tool_results_merge_into_one_user_message() {
        let req = ChatRequest {
            model: "claude-test".into(),
            system: Some("sys".into()),
            messages: vec![
                Message::User {
                    content: "go".into(),
                },
                Message::Assistant {
                    content: Some("calling".into()),
                    tool_calls: vec![
                        ToolCall {
                            id: "a".into(),
                            name: "t1".into(),
                            arguments: json!({}),
                        },
                        ToolCall {
                            id: "b".into(),
                            name: "t2".into(),
                            arguments: json!({}),
                        },
                    ],
                },
                Message::ToolResult {
                    tool_call_id: "a".into(),
                    name: "t1".into(),
                    content: "r1".into(),
                    is_error: false,
                },
                Message::ToolResult {
                    tool_call_id: "b".into(),
                    name: "t2".into(),
                    content: "r2".into(),
                    is_error: true,
                },
            ],
            tools: vec![],
            max_tokens: 10,
            temperature: None,
        };
        let messages = build_messages(&req);
        assert_eq!(messages.len(), 3); // user, assistant, merged tool-result user
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[1]["is_error"], true);
        // System prompt is top-level, not a message.
        let body = build_body(&req);
        assert_eq!(body["system"], "sys");
    }

    #[test]
    fn parses_interleaved_text_and_tool_use() {
        let body = json!({
            "content": [
                { "type": "text", "text": "thinking… " },
                { "type": "tool_use", "id": "tu_1", "name": "lookup", "input": { "q": "eth" } }
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 7, "output_tokens": 3 }
        });
        let resp = parse_response(body).unwrap();
        assert_eq!(resp.content.as_deref(), Some("thinking… "));
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert_eq!(resp.tool_calls[0].arguments["q"], "eth");
    }
}
