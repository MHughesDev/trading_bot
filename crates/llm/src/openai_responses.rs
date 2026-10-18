//! OpenAI Responses adapter (`POST /v1/responses`).
//!
//! Newer OpenAI reasoning models refuse function tools on chat-completions
//! (seen live with `gpt-6.1-sol` on 2026-10-05: "Function tools with
//! reasoning_effort are not supported … use /v1/responses", and `"none"` is not
//! an accepted effort for that model either). The Responses API is the only
//! endpoint on which those models will call a tool, so the OpenAI flavour of
//! [`crate::openai::chat`] routes here when a model needs it.
//!
//! Wire mapping from the crate's [`ChatRequest`]:
//! - `system` → `instructions`
//! - user / assistant text → `input` items `{role, content}`
//! - prior assistant tool calls → `{type:"function_call", call_id, name, arguments}`
//!   with `arguments` as a JSON **string**, the call id being exactly the id the
//!   model returned (it is what [`ToolCall::id`] holds) so the round trip pairs
//! - tool results → `{type:"function_call_output", call_id, output}`
//! - tools → the flat `{type:"function", name, description, parameters}` shape
//! - `max_tokens` → `max_output_tokens`
//! - `reasoning: {effort: "low"}` — an agent step is one small decision; low keeps
//!   cost and latency down and is an accepted value on every reasoning model seen
//! - no `temperature`: every model routed here is a reasoning model, and those
//!   refuse a non-default temperature
//! - `store: false`: the transcript is the caller's; nothing is kept server-side,
//!   and nothing relies on `previous_response_id`

use serde_json::{json, Map, Value};

use crate::error::{classify_status, truncate, LlmError};
use crate::openai::{classify_openai_400, post_json};
use crate::types::{ChatRequest, ChatResponse, Message, StopReason, ToolCall, ToolChoice, Usage};

/// The reasoning effort sent on every Responses call. See the module docs.
pub(crate) const REASONING_EFFORT: &str = "low";

fn build_input(req: &ChatRequest) -> Vec<Value> {
    let mut out = Vec::new();
    for msg in &req.messages {
        match msg {
            Message::User { content } => {
                out.push(json!({ "role": "user", "content": content }));
            }
            Message::Assistant {
                content,
                tool_calls,
            } => {
                // An assistant turn that said nothing carries no information; an
                // empty message item is noise the server may reject.
                if let Some(text) = content.as_deref().filter(|t| !t.is_empty()) {
                    out.push(json!({ "role": "assistant", "content": text }));
                }
                for call in tool_calls {
                    out.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": serde_json::to_string(&call.arguments)
                            .unwrap_or_else(|_| "{}".into()),
                    }));
                }
            }
            Message::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } => {
                let output = if *is_error {
                    format!("ERROR: {content}")
                } else {
                    content.clone()
                };
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": output,
                }));
            }
        }
    }
    out
}

/// The Responses request body for `req`.
pub(crate) fn build_body(req: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    if let Some(system) = req.system.as_deref().filter(|s| !s.is_empty()) {
        body.insert("instructions".into(), json!(system));
    }
    body.insert("input".into(), Value::Array(build_input(req)));
    body.insert("max_output_tokens".into(), json!(req.max_tokens));
    body.insert("reasoning".into(), json!({ "effort": REASONING_EFFORT }));
    body.insert("store".into(), json!(false));

    if let Some(schema) = &req.schema {
        body.insert(
            "text".into(),
            json!({
                "format": {
                    "type": "json_schema",
                    "name": "tool_call",
                    "schema": schema,
                    "strict": crate::openai::strict_compatible(schema),
                }
            }),
        );
    }

    if !req.tools.is_empty() {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        if let Some(choice) = &req.tool_choice {
            body.insert(
                "tool_choice".into(),
                match choice {
                    ToolChoice::Auto => json!("auto"),
                    ToolChoice::Required => json!("required"),
                    ToolChoice::None => json!("none"),
                    ToolChoice::Tool(name) => json!({"type": "function", "name": name}),
                },
            );
        }
    }
    Value::Object(body)
}

/// Parse a Responses body into the crate's response type.
pub(crate) fn parse_response(body: Value) -> Result<ChatResponse, LlmError> {
    if body.get("status").and_then(Value::as_str) == Some("failed") {
        let detail = body
            .get("error")
            .map(Value::to_string)
            .unwrap_or_else(|| "response failed".into());
        return Err(LlmError::Provider {
            status: 200,
            body: truncate(&detail),
        });
    }
    let output = body
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| LlmError::Deserialize("no output in response".into()))?;

    let mut texts: Vec<String> = Vec::new();
    let mut tool_calls = Vec::new();
    for (i, item) in output.iter().enumerate() {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                if let Some(parts) = item.get("content").and_then(Value::as_array) {
                    for part in parts {
                        if part.get("type").and_then(Value::as_str) == Some("output_text") {
                            if let Some(t) = part.get("text").and_then(Value::as_str) {
                                texts.push(t.to_string());
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                // `call_id` is what a `function_call_output` must name; `id` is
                // the output item's own id and is NOT accepted there.
                let id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{i}"));
                let raw_args = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let arguments: Value = serde_json::from_str(raw_args).map_err(|e| {
                    LlmError::Deserialize(format!(
                        "tool call '{name}' has malformed arguments ({e}): {}",
                        truncate(raw_args)
                    ))
                })?;
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
            // `reasoning` items (and anything newer) carry nothing the caller uses.
            _ => {}
        }
    }

    let content = Some(texts.concat()).filter(|s| !s.is_empty());
    let incomplete_for_tokens = body.get("status").and_then(Value::as_str) == Some("incomplete")
        && body
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            == Some("max_output_tokens");
    let stop_reason = if !tool_calls.is_empty() {
        StopReason::ToolUse
    } else if incomplete_for_tokens {
        StopReason::MaxTokens
    } else {
        StopReason::EndTurn
    };

    let cached = body
        .pointer("/usage/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input = body
        .pointer("/usage/input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let usage = Usage {
        // As on chat-completions, the cached portion is inside `input_tokens`;
        // subtract it so `Usage::prompt_tokens` sums to the prompt.
        input_tokens: input.saturating_sub(cached),
        output_tokens: body
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_input_tokens: cached,
        cache_creation_input_tokens: 0,
    };

    Ok(ChatResponse {
        content,
        tool_calls,
        stop_reason,
        usage,
    })
}

/// One Responses call. Errors are classified like chat-completions errors; the
/// caller scrubs the key from them.
pub(crate) async fn chat(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    req: &ChatRequest,
) -> Result<ChatResponse, LlmError> {
    let body = build_body(req);
    let (status, retry_after, text) =
        post_json(http, &format!("{base_url}/v1/responses"), api_key, &body).await?;
    if status == 400 {
        if let Some(err) = classify_openai_400(&text) {
            return Err(err);
        }
    }
    if !(200..300).contains(&status) {
        return Err(classify_status(status, &text, retry_after));
    }
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| LlmError::Deserialize(e.to_string()))?;
    parse_response(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolDef;

    #[test]
    fn the_tool_choice_for_one_tool_is_flat() {
        let req = ChatRequest {
            model: "m".into(),
            tools: vec![ToolDef {
                name: "lookup".into(),
                description: "d".into(),
                input_schema: json!({"type": "object"}),
            }],
            tool_choice: Some(ToolChoice::Tool("lookup".into())),
            temperature: Some(0.0),
            ..ChatRequest::default()
        };
        let body = build_body(&req);
        assert_eq!(body["tool_choice"]["name"], "lookup");
        assert!(body.get("temperature").is_none());
        assert_eq!(body["tools"][0]["name"], "lookup");
        assert!(body["tools"][0].get("function").is_none());
    }

    #[test]
    fn a_truncated_answer_is_max_tokens() {
        let body = json!({
            "status": "incomplete",
            "incomplete_details": {"reason": "max_output_tokens"},
            "output": [{"type": "reasoning", "summary": []}],
        });
        assert_eq!(
            parse_response(body).unwrap().stop_reason,
            StopReason::MaxTokens
        );
    }

    #[test]
    fn a_failed_response_is_an_error() {
        let body = json!({"status": "failed", "error": {"message": "boom"}, "output": []});
        assert!(parse_response(body).is_err());
    }
}
