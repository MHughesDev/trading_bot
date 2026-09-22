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
use crate::types::{
    ChatRequest, ChatResponse, Message, ModelInfo, StopReason, ToolCall, ToolChoice, Usage,
};

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

/// The marker that closes a cache prefix segment.
fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// Put a `cache_control` marker on the last content block of the last message.
///
/// A plain-string user message is lifted into a single text block first, because
/// `cache_control` is a property of a block and a string has none.
///
/// This is the third and last breakpoint, and it is the one that moves: every
/// turn appends, so next call's marker sits further along the same prefix and
/// everything in front of it is read from the cache written here.
fn mark_last_message(messages: &mut [Value]) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    let content = match last.get_mut("content") {
        Some(c) => c,
        None => return,
    };
    if let Some(text) = content.as_str() {
        *content = json!([{ "type": "text", "text": text, "cache_control": ephemeral() }]);
        return;
    }
    if let Some(block) = content.as_array_mut().and_then(<[Value]>::last_mut) {
        if let Some(obj) = block.as_object_mut() {
            obj.insert("cache_control".into(), ephemeral());
        }
    }
}

/// Build the request body, with the three cache breakpoints of AGENT-005 §6.1.
///
/// **Prefix order, stable-first: tools → system → messages.** That is the order
/// the provider hashes the prefix in, so a breakpoint is only worth placing
/// after everything in front of it is byte-identical to the previous call. A
/// breakpoint placed after something that varies never hits; a breakpoint placed
/// before something stable leaves those bytes uncached. Both are silent — the
/// request succeeds either way and the cost shows up only in `Usage`.
///
/// Segments under the provider's minimum (1,024 tokens) are ignored by the API
/// rather than rejected, so this is safe on a prompt too small to benefit.
fn build_body(req: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("max_tokens".into(), json!(req.max_tokens));

    // Breakpoint 3 (last in prefix order, applied here because `messages` is
    // built here): the last content block of the last message.
    let mut messages = build_messages(req);
    mark_last_message(&mut messages);
    body.insert("messages".into(), Value::Array(messages));

    // Breakpoint 2: `system` as a text block rather than a bare string. The
    // charter is byte-stable (AGENT-005 §3.1), so this segment is the one that
    // survives an entire conversation.
    if let Some(system) = &req.system {
        body.insert(
            "system".into(),
            json!([{ "type": "text", "text": system, "cache_control": ephemeral() }]),
        );
    }
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if !req.tools.is_empty() {
        let last = req.tools.len() - 1;
        let tools: Vec<Value> = req
            .tools
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut def = json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                });
                // Breakpoint 1: the LAST tool closes the whole tool layer. One
                // marker, not one per tool — a breakpoint per tool would spend
                // the provider's four-breakpoint allowance on a layer that is
                // append-only anyway.
                if i == last {
                    def.as_object_mut()
                        .expect("tool definition is an object")
                        .insert("cache_control".into(), ephemeral());
                }
                def
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if let Some(choice) = &req.tool_choice {
            body.insert(
                "tool_choice".into(),
                match choice {
                    ToolChoice::Auto => json!({"type": "auto"}),
                    // Anthropic spells "you must call something" as `any`.
                    ToolChoice::Required => json!({"type": "any"}),
                    ToolChoice::None => json!({"type": "none"}),
                    ToolChoice::Tool(name) => json!({"type": "tool", "name": name}),
                },
            );
        }
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

    let u = |field: &str| {
        body.pointer(&format!("/usage/{field}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let usage = Usage {
        input_tokens: u("input_tokens"),
        output_tokens: u("output_tokens"),
        cache_read_input_tokens: u("cache_read_input_tokens"),
        cache_creation_input_tokens: u("cache_creation_input_tokens"),
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
            schema: None,
            num_ctx: None,
            keep_alive: None,
            tool_choice: None,
        };
        let messages = build_messages(&req);
        assert_eq!(messages.len(), 3); // user, assistant, merged tool-result user
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[1]["is_error"], true);
        // System prompt is top-level, not a message. Since AGENT-005 §6.1 it
        // is a one-element block array rather than a bare string, because a
        // `cache_control` marker is a property of a block.
        let body = build_body(&req);
        assert_eq!(body["system"][0]["text"], "sys");
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
