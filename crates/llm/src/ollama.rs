//! Ollama adapter (`POST /api/chat`, non-streaming).
//!
//! Wire quirks handled here:
//! - no auth by default (local daemon); base_url selects the host
//! - tool-call `arguments` arrive as a JSON **object** (unlike OpenAI)
//! - tool calls carry **no ids** — we synthesize `call_N` and results are
//!   paired positionally (`role:"tool"` messages in order)
//! - models without tool support return an error string we map to
//!   `ToolsUnsupported`

use serde_json::{json, Map, Value};

use crate::error::{classify_status, truncate, LlmError};
use crate::types::{ChatRequest, ChatResponse, Message, ModelInfo, StopReason, ToolCall, Usage};

fn build_messages(req: &ChatRequest) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(system) = &req.system {
        out.push(json!({ "role": "system", "content": system }));
    }
    for msg in &req.messages {
        match msg {
            Message::User { content } => out.push(json!({ "role": "user", "content": content })),
            Message::Assistant {
                content,
                tool_calls,
            } => {
                let mut m = Map::new();
                m.insert("role".into(), json!("assistant"));
                m.insert("content".into(), json!(content.clone().unwrap_or_default()));
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                                "function": { "name": c.name, "arguments": c.arguments }
                            })
                        })
                        .collect();
                    m.insert("tool_calls".into(), Value::Array(calls));
                }
                out.push(Value::Object(m));
            }
            Message::ToolResult {
                content, is_error, ..
            } => {
                // Ollama's tool role has no id field; pairing is positional.
                let text = if *is_error {
                    format!("ERROR: {content}")
                } else {
                    content.clone()
                };
                out.push(json!({ "role": "tool", "content": text }));
            }
        }
    }
    out
}

fn build_body(req: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("messages".into(), Value::Array(build_messages(req)));
    body.insert("stream".into(), json!(false));
    let mut options = Map::new();
    options.insert("num_predict".into(), json!(req.max_tokens));
    if let Some(t) = req.temperature {
        options.insert("temperature".into(), json!(t));
    }
    body.insert("options".into(), Value::Object(options));
    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    Value::Object(body)
}

fn parse_response(body: Value) -> Result<ChatResponse, LlmError> {
    let message = body
        .get("message")
        .ok_or_else(|| LlmError::Deserialize("no message in response".into()))?;

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|c| c.as_array()) {
        for (i, call) in calls.iter().enumerate() {
            let function = call.get("function").cloned().unwrap_or(Value::Null);
            tool_calls.push(ToolCall {
                // Ollama sends no ids — synthesize stable positional ones.
                id: format!("call_{i}"),
                name: function
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                arguments: function.get("arguments").cloned().unwrap_or(json!({})),
            });
        }
    }

    let done_reason = body
        .get("done_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("stop");
    let stop_reason = if !tool_calls.is_empty() {
        StopReason::ToolUse
    } else {
        match done_reason {
            "stop" => StopReason::EndTurn,
            "length" => StopReason::MaxTokens,
            other => StopReason::Other(other.to_string()),
        }
    };

    let usage = Usage {
        input_tokens: body
            .get("prompt_eval_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: body.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0),
    };

    Ok(ChatResponse {
        content,
        tool_calls,
        stop_reason,
        usage,
    })
}

pub async fn chat(
    http: &reqwest::Client,
    base_url: &str,
    req: &ChatRequest,
) -> Result<ChatResponse, LlmError> {
    let resp = http
        .post(format!("{base_url}/api/chat"))
        .json(&build_body(req))
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;

    let lower = text.to_lowercase();
    if lower.contains("does not support tools") {
        return Err(LlmError::ToolsUnsupported(truncate(&text)));
    }
    if !(200..300).contains(&status) {
        return Err(classify_status(status, &text, None));
    }

    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| LlmError::Deserialize(e.to_string()))?;
    parse_response(parsed)
}

pub async fn list_models(
    http: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<ModelInfo>, LlmError> {
    let resp = http
        .get(format!("{base_url}/api/tags"))
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
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|v| v.as_str()))
                .map(|name| ModelInfo {
                    id: name.to_string(),
                    display_name: None,
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_calls_get_synthesized_ids_and_object_arguments() {
        let body = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    { "function": { "name": "lookup", "arguments": { "q": "sol" } } },
                    { "function": { "name": "other", "arguments": { "n": 2 } } }
                ]
            },
            "done_reason": "stop",
            "prompt_eval_count": 12,
            "eval_count": 4
        });
        let resp = parse_response(body).unwrap();
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert_eq!(resp.tool_calls[0].id, "call_0");
        assert_eq!(resp.tool_calls[1].id, "call_1");
        assert_eq!(resp.tool_calls[0].arguments["q"], "sol");
    }

    #[test]
    fn tool_results_map_to_positional_tool_messages() {
        let req = ChatRequest {
            model: "qwen".into(),
            system: None,
            messages: vec![
                Message::User {
                    content: "go".into(),
                },
                Message::Assistant {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_0".into(),
                        name: "t".into(),
                        arguments: json!({}),
                    }],
                },
                Message::ToolResult {
                    tool_call_id: "call_0".into(),
                    name: "t".into(),
                    content: "out".into(),
                    is_error: false,
                },
            ],
            tools: vec![],
            max_tokens: 5,
            temperature: None,
        };
        let messages = build_messages(&req);
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["content"], "out");
        assert!(messages[2].get("tool_call_id").is_none());
    }
}
