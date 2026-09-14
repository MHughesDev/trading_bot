//! OpenAI chat-completions adapter (`POST /v1/chat/completions`).
//!
//! Wire quirks handled here:
//! - tool definitions wrap in `{type:"function", function:{...}}`
//! - response `tool_calls[].function.arguments` is a **JSON string** — parsed
//!   at this boundary so callers always see structured arguments
//! - newer models take `max_completion_tokens`; older ones only `max_tokens`.
//!   We send the former and retry once with the latter on the specific 400.

use serde_json::{json, Map, Value};

use crate::error::{classify_status, truncate, LlmError};
use crate::types::{
    ChatRequest, ChatResponse, Message, ModelInfo, StopReason, ToolCall, ToolChoice, Usage,
};

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
                m.insert(
                    "content".into(),
                    content.as_deref().map(Value::from).unwrap_or(Value::Null),
                );
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    // OpenAI expects stringified arguments.
                                    "arguments": serde_json::to_string(&c.arguments)
                                        .unwrap_or_else(|_| "{}".into()),
                                }
                            })
                        })
                        .collect();
                    m.insert("tool_calls".into(), Value::Array(calls));
                }
                out.push(Value::Object(m));
            }
            Message::ToolResult {
                tool_call_id,
                content,
                is_error,
                ..
            } => {
                let text = if *is_error {
                    format!("ERROR: {content}")
                } else {
                    content.clone()
                };
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": tool_call_id,
                    "content": text,
                }));
            }
        }
    }
    out
}

/// Which OpenAI-compatible server is on the far end.
///
/// The wire format is shared; two details are not, and both are load-bearing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    /// api.openai.com and anything that matches it.
    OpenAi,
    /// vLLM's `--api-server`. Wants `max_tokens`, and — the reason it is the
    /// production local backend — applies `guided_json` to the arguments while the
    /// model's own tool template still runs.
    Vllm,
}

fn build_body(req: &ChatRequest, use_completion_tokens: bool, flavor: Flavor) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("messages".into(), Value::Array(build_messages(req)));
    let tokens_key = if use_completion_tokens {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    body.insert(tokens_key.into(), json!(req.max_tokens));
    if let Some(t) = req.temperature {
        body.insert("temperature".into(), json!(t));
    }

    // Grammar-constrained decoding (harness guide §3.1). The two servers disagree
    // about the field name, and the disagreement is not symmetric:
    //
    // - `response_format: {type: json_schema, …}` is the OpenAI spelling, which vLLM
    //   also accepts on recent versions. Safe to send to both.
    // - `guided_json` is vLLM's own, routed to its xgrammar/outlines backend. It goes
    //   ONLY to vLLM, because api.openai.com rejects unrecognised body parameters
    //   with a 400 rather than ignoring them — so "send both spellings and let the
    //   server pick" fails every OpenAI request that asks for a schema.
    if let Some(schema) = &req.schema {
        body.insert(
            "response_format".into(),
            json!({
                "type": "json_schema",
                "json_schema": { "name": "tool_call", "schema": schema, "strict": true }
            }),
        );
        if flavor == Flavor::Vllm {
            body.insert("guided_json".into(), schema.clone());
        }
        // Unlike Ollama, these servers do not make the caller choose: the grammar and
        // a tool list can both be present, and `tools` is left to the caller rather
        // than dropped here. The harness's local path deliberately sends the grammar
        // alone — under the two-step decode the grammar *is* the tool interface, and
        // a tool list would be a second, weaker statement of the same thing.
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
        if let Some(choice) = &req.tool_choice {
            body.insert(
                "tool_choice".into(),
                match choice {
                    ToolChoice::Auto => json!("auto"),
                    ToolChoice::Required => json!("required"),
                    ToolChoice::None => json!("none"),
                    ToolChoice::Tool(name) => {
                        json!({"type": "function", "function": {"name": name}})
                    }
                },
            );
        }
    }
    Value::Object(body)
}

fn classify_openai_400(body: &str) -> Option<LlmError> {
    let lower = body.to_lowercase();
    if lower.contains("context_length") || lower.contains("maximum context length") {
        return Some(LlmError::ContextTooLarge(truncate(body)));
    }
    None
}

fn parse_response(body: Value) -> Result<ChatResponse, LlmError> {
    let choice = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| LlmError::Deserialize("no choices in response".into()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| LlmError::Deserialize("choice has no message".into()))?;

    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|c| c.as_array()) {
        for (i, call) in calls.iter().enumerate() {
            let id = call
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("call_{i}"));
            let function = call.get("function").cloned().unwrap_or(Value::Null);
            let name = function
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let raw_args = function
                .get("arguments")
                .and_then(|v| v.as_str())
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
    }

    let finish = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let stop_reason = match finish {
        "stop" => StopReason::EndTurn,
        "tool_calls" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    };

    let usage = Usage {
        input_tokens: body
            .pointer("/usage/prompt_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: body
            .pointer("/usage/completion_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    };

    Ok(ChatResponse {
        content,
        tool_calls,
        stop_reason,
        usage,
    })
}

async fn post_chat(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    body: &Value,
) -> Result<(u16, Option<u64>, String), LlmError> {
    let resp = http
        .post(format!("{base_url}/v1/chat/completions"))
        .header("authorization", format!("Bearer {api_key}"))
        .json(body)
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
    Ok((status, retry_after, text))
}

pub async fn chat(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    req: &ChatRequest,
    flavor: Flavor,
) -> Result<ChatResponse, LlmError> {
    // vLLM takes `max_tokens`. Asking it for `max_completion_tokens` first would
    // spend a round trip to learn something already known.
    let completion_tokens = flavor == Flavor::OpenAi;
    let mut body = build_body(req, completion_tokens, flavor);
    let (mut status, mut retry_after, mut text) = post_chat(http, base_url, api_key, &body).await?;

    // Older models reject max_completion_tokens; retry once with max_tokens.
    if status == 400 && text.contains("max_completion_tokens") {
        body = build_body(req, false, flavor);
        (status, retry_after, text) = post_chat(http, base_url, api_key, &body).await?;
    }

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

/// Model ids that are not chat models and should be hidden from the picker.
const NON_CHAT_MARKERS: &[&str] = &[
    "embedding",
    "whisper",
    "tts",
    "dall-e",
    "audio",
    "realtime",
    "moderation",
    "transcribe",
    "image",
];

pub async fn list_models(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, LlmError> {
    let resp = http
        .get(format!("{base_url}/v1/models"))
        .header("authorization", format!("Bearer {api_key}"))
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
    let mut models: Vec<ModelInfo> = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                .filter(|id| {
                    let lower = id.to_lowercase();
                    (lower.starts_with("gpt") || lower.starts_with('o'))
                        && !NON_CHAT_MARKERS.iter().any(|m| lower.contains(m))
                })
                .map(|id| ModelInfo {
                    id: id.to_string(),
                    display_name: None,
                })
                .collect()
        })
        .unwrap_or_default();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolDef;

    fn req_with_tools() -> ChatRequest {
        ChatRequest {
            model: "gpt-test".into(),
            system: Some("be brief".into()),
            messages: vec![
                Message::User {
                    content: "hi".into(),
                },
                Message::Assistant {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".into(),
                        name: "lookup".into(),
                        arguments: serde_json::json!({"q": "x"}),
                    }],
                },
                Message::ToolResult {
                    tool_call_id: "call_1".into(),
                    name: "lookup".into(),
                    content: "42".into(),
                    is_error: false,
                },
            ],
            tools: vec![ToolDef {
                name: "lookup".into(),
                description: "look things up".into(),
                input_schema: serde_json::json!({"type":"object","properties":{}}),
            }],
            max_tokens: 100,
            temperature: Some(0.2),
            schema: None,
            num_ctx: None,
            keep_alive: None,
            tool_choice: None,
        }
    }

    #[test]
    fn wire_shape_matches_openai() {
        let body = build_body(&req_with_tools(), true, Flavor::OpenAi);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        // Assistant tool call arguments are stringified on the wire.
        assert!(body["messages"][2]["tool_calls"][0]["function"]["arguments"].is_string());
        assert_eq!(body["messages"][3]["role"], "tool");
        assert_eq!(body["messages"][3]["tool_call_id"], "call_1");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["max_completion_tokens"], 100);
        assert!(build_body(&req_with_tools(), false, Flavor::OpenAi)
            .get("max_tokens")
            .is_some());
    }

    fn req_with_schema() -> ChatRequest {
        let mut r = req_with_tools();
        r.schema = Some(serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string", "enum": ["a", "b"]}},
            "required": ["name"],
        }));
        r
    }

    /// The production local backend gets vLLM's own spelling, which is the one that
    /// actually reaches its xgrammar/outlines path.
    #[test]
    fn vllm_gets_guided_json_as_well_as_response_format() {
        let body = build_body(&req_with_schema(), false, Flavor::Vllm);
        assert_eq!(body["guided_json"]["properties"]["name"]["enum"][0], "a");
        assert_eq!(body["response_format"]["type"], "json_schema");
    }

    /// api.openai.com rejects unrecognised body parameters with a 400 rather than
    /// ignoring them, so "send both spellings and let the server pick" would fail
    /// every OpenAI request that asks for a schema.
    #[test]
    fn openai_never_sees_vllms_private_parameter() {
        let body = build_body(&req_with_schema(), true, Flavor::OpenAi);
        assert!(
            body.get("guided_json").is_none(),
            "an unknown parameter is a 400 on OpenAI, not a no-op"
        );
        assert_eq!(body["response_format"]["type"], "json_schema");
    }

    /// Unlike Ollama, these servers do not make the caller choose between a grammar
    /// and a tool list — so the adapter must not silently drop one.
    #[test]
    fn a_schema_does_not_suppress_the_tool_list_here() {
        let body = build_body(&req_with_schema(), false, Flavor::Vllm);
        assert!(body.get("tools").is_some());
    }

    #[test]
    fn forcing_one_tool_is_expressed_on_the_wire() {
        let mut r = req_with_tools();
        r.tool_choice = Some(ToolChoice::Tool("lookup".into()));
        let body = build_body(&r, true, Flavor::OpenAi);
        assert_eq!(body["tool_choice"]["function"]["name"], "lookup");

        r.tool_choice = Some(ToolChoice::Required);
        assert_eq!(
            build_body(&r, true, Flavor::OpenAi)["tool_choice"],
            "required"
        );
    }

    #[test]
    fn parses_tool_call_response_with_string_arguments() {
        let body = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_9",
                        "type": "function",
                        "function": { "name": "lookup", "arguments": "{\"q\":\"btc\"}" }
                    }]
                }
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        });
        let resp = parse_response(body).unwrap();
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert_eq!(resp.tool_calls[0].arguments["q"], "btc");
        assert_eq!(resp.usage.input_tokens, 10);
    }

    #[test]
    fn malformed_arguments_surface_as_deserialize_error() {
        let body = serde_json::json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "tool_calls": [{
                        "id": "call_9",
                        "function": { "name": "lookup", "arguments": "{not json" }
                    }]
                }
            }]
        });
        assert!(matches!(
            parse_response(body),
            Err(LlmError::Deserialize(_))
        ));
    }
}
