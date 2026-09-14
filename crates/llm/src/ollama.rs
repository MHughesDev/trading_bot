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

    // `num_ctx` is NOT optional, and omitting it is a silent correctness bug.
    //
    // Ollama defaults the context window to the Modelfile's value — commonly 4096,
    // sometimes 2048 — regardless of what the model advertises. Send a 24k-token
    // prompt without this and Ollama truncates it from the LEFT: the system charter
    // and the tool schemas go first, and what arrives looks like the model ignoring
    // its instructions rather than like a transport that quietly dropped them.
    //
    // The harness has already decided this number (`effective_budget_tokens`) and
    // refuses to assemble a prompt over it, so sending it here makes the two agree
    // rather than leaving the backend to guess.
    if let Some(ctx) = req.num_ctx {
        options.insert("num_ctx".into(), json!(ctx));
    }
    body.insert("options".into(), Value::Object(options));

    // Keep the weights resident between steps. A ~21 GB model evicted after each
    // call reloads from NVMe on the next one — measured at ~100 s on the dev box,
    // which would be paid once per step of a multi-step session.
    if let Some(keep) = &req.keep_alive {
        body.insert("keep_alive".into(), json!(keep));
    }

    // Grammar-constrained decoding (harness guide §3.1). Ollama takes a JSON Schema
    // in `format` and honours it at sampling time.
    //
    // `format` and `tools` DO NOT COMPOSE — measured on 0.33.3, passing both makes
    // `format` win and `tool_calls` come back null. So when a schema is set we send
    // it and deliberately omit `tools`: a caller that asked for a grammar gets the
    // grammar, and silently sending both would produce a response shape neither the
    // caller nor the parser expects. See docs/LOCAL_TIER_FINDINGS.md §2.
    if let Some(schema) = &req.schema {
        body.insert("format".into(), schema.clone());
        return Value::Object(body);
    }

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

    // Status first, THEN the string sniff.
    //
    // The sniff used to run before this check, which meant a successful 200 whose
    // assistant *content* happened to contain "does not support tools" — an entirely
    // plausible sentence for a research agent discussing model capabilities — was
    // reported as a hard ToolsUnsupported error. Only a non-2xx body is Ollama
    // speaking; a 2xx body is the model speaking, and the two must not be read the
    // same way.
    if !(200..300).contains(&status) {
        if text.to_lowercase().contains("does not support tools") {
            return Err(LlmError::ToolsUnsupported(truncate(&text)));
        }
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

/// Models the backend currently holds **in memory** (`GET /api/ps`).
///
/// Distinct from [`list_models`], which lists what is available to load. The
/// difference decides a real question: a fit check that compares a model against
/// FREE device memory will refuse a model that is already resident, because the
/// memory it occupies reads as used. Asking which models are loaded is what turns
/// that false refusal back into the correct answer — the weights are on the card, so
/// they fit on the card.
///
/// A backend that does not answer is not an error. An empty list means "cannot say",
/// and the caller falls back to the ordinary memory check.
pub async fn resident_models(http: &reqwest::Client, base_url: &str) -> Vec<String> {
    let Ok(resp) = http.get(format!("{base_url}/api/ps")).send().await else {
        return Vec::new();
    };
    if !resp.status().is_success() {
        return Vec::new();
    }
    let Ok(body) = resp.json::<Value>().await else {
        return Vec::new();
    };
    body.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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
            schema: None,
            num_ctx: None,
            keep_alive: None,
            tool_choice: None,
        };
        let messages = build_messages(&req);
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["content"], "out");
        assert!(messages[2].get("tool_call_id").is_none());
    }
}

#[cfg(test)]
mod local_tier_wire_tests {
    use super::*;
    use crate::types::{ChatRequest, Message};

    fn req() -> ChatRequest {
        ChatRequest {
            model: "qwen3.6-35b-a3b".into(),
            system: Some("charter".into()),
            messages: vec![Message::User {
                content: "go".into(),
            }],
            tools: vec![],
            max_tokens: 512,
            temperature: Some(0.0),
            schema: None,
            num_ctx: None,
            keep_alive: None,
            tool_choice: None,
        }
    }

    /// The silent-truncation bug, pinned.
    ///
    /// Without `num_ctx`, Ollama uses the Modelfile default (often 4096) and cuts a
    /// longer prompt from the LEFT — taking the system charter and the tool schemas
    /// first. The symptom is a model that appears to ignore its instructions.
    #[test]
    fn the_context_window_is_sent_so_the_backend_cannot_silently_truncate() {
        let mut r = req();
        r.num_ctx = Some(24_000);
        let body = build_body(&r);
        assert_eq!(
            body["options"]["num_ctx"], 24_000,
            "the harness knows the budget; the backend must be told it"
        );
    }

    #[test]
    fn keep_alive_is_sent_so_a_large_model_is_not_reloaded_every_step() {
        let mut r = req();
        r.keep_alive = Some("30m".into());
        assert_eq!(build_body(&r)["keep_alive"], "30m");
    }

    #[test]
    fn omitting_them_sends_nothing_rather_than_a_guess() {
        let body = build_body(&req());
        assert!(body["options"].get("num_ctx").is_none());
        assert!(body.get("keep_alive").is_none());
    }

    /// Measured behaviour, encoded: `format` and `tools` do not compose on Ollama —
    /// passing both makes `format` win and `tool_calls` come back null. So a request
    /// carrying a schema sends the schema and omits the tools, rather than sending
    /// both and getting a response shape the parser does not expect.
    #[test]
    fn a_schema_request_omits_tools_because_they_do_not_compose() {
        let mut r = req();
        r.tools = vec![crate::types::ToolDef {
            name: "read_bars".into(),
            description: "Read bars.".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        r.schema = Some(serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        }));
        let body = build_body(&r);
        assert!(body.get("format").is_some(), "the grammar is sent");
        assert!(
            body.get("tools").is_none(),
            "sending both would make format win silently and null out tool_calls"
        );
    }

    #[test]
    fn without_a_schema_tools_are_sent_normally() {
        let mut r = req();
        r.tools = vec![crate::types::ToolDef {
            name: "read_bars".into(),
            description: "Read bars.".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let body = build_body(&r);
        assert!(body.get("format").is_none());
        assert_eq!(body["tools"][0]["function"]["name"], "read_bars");
    }
}
