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
                // `null` content is accepted only beside tool calls. An assistant
                // turn that said nothing and called nothing is a 400 on
                // api.openai.com ("expected a string, got null"), so it goes on
                // the wire as the empty string it means.
                let content_value = match content.as_deref() {
                    Some(text) => Value::from(text),
                    None if tool_calls.is_empty() => Value::from(""),
                    None => Value::Null,
                };
                m.insert("content".into(), content_value);
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

/// Whether `schema` is inside OpenAI's strict structured-output subset: every
/// object sets `additionalProperties: false` and lists every property as required.
pub(crate) fn strict_compatible(schema: &Value) -> bool {
    match schema {
        Value::Object(map) => {
            let is_object = map.get("type").and_then(Value::as_str) == Some("object")
                || map.contains_key("properties");
            if is_object {
                if map.get("additionalProperties") != Some(&Value::Bool(false)) {
                    return false;
                }
                let props: Vec<&String> = map
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|p| p.keys().collect())
                    .unwrap_or_default();
                let required: Vec<&str> = map
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|r| r.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                if !props.iter().all(|p| required.contains(&p.as_str())) {
                    return false;
                }
            }
            map.values().all(strict_compatible)
        }
        Value::Array(items) => items.iter().all(strict_compatible),
        _ => true,
    }
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
                "json_schema": {
                    "name": "tool_call",
                    "schema": schema,
                    // api.openai.com refuses `strict: true` with a 400 unless every
                    // object closes `additionalProperties` and requires every
                    // property. A schema outside that subset still constrains the
                    // output non-strictly rather than failing the call; vLLM gets
                    // the same flag and enforces through `guided_json` regardless.
                    "strict": strict_compatible(schema),
                }
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

/// Models that refused a `temperature` this process sent them.
///
/// OpenAI's reasoning models (the o-series and the GPT-5 family) accept only the
/// default temperature and answer anything else, including the `0.0` every
/// frontier profile asks for on tool calls, with a 400 naming the parameter. The
/// first call learns it and retries without; this set is what stops every later
/// call paying the same refused round trip. Keyed by `base_url` and model, because
/// a vLLM server and api.openai.com may serve models with the same name.
fn temperature_refusers() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Models that refused function tools while reasoning was on.
///
/// Newer OpenAI reasoning models (seen with `gpt-6.1-sol`, 2026-10-05) answer a
/// chat-completions request that carries tools with a 400 telling the caller to
/// use `/v1/responses` or set `reasoning_effort` to `"none"`. The first call
/// learns it and retries with `"none"`; this set stops every later call paying
/// for the same refusal.
fn reasoning_tool_refusers() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Whether a 400 body is the server refusing tools alongside reasoning.
fn refuses_tools_with_reasoning(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("reasoning_effort") && lower.contains("tools")
}

fn refusal_key(base_url: &str, model: &str) -> String {
    format!("{base_url} {model}")
}

/// Whether a 400 body is the server refusing the `temperature` parameter.
fn refuses_temperature(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("temperature")
        && (lower.contains("unsupported") || lower.contains("not support"))
}

pub(crate) fn classify_openai_400(body: &str) -> Option<LlmError> {
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
        // OpenAI reports the cached portion *inside* `prompt_tokens` rather than
        // beside it, so this is subtracted below rather than added — the sum has to
        // come out as the prompt on every provider or `Usage::prompt_tokens` means
        // something different depending on who answered.
        cache_read_input_tokens: body
            .pointer("/usage/prompt_tokens_details/cached_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        // OpenAI's cache is implicit: there is no creation charge to report.
        cache_creation_input_tokens: 0,
    };
    let usage = Usage {
        input_tokens: usage
            .input_tokens
            .saturating_sub(usage.cache_read_input_tokens),
        ..usage
    };

    Ok(ChatResponse {
        content,
        tool_calls,
        stop_reason,
        usage,
    })
}

pub(crate) async fn post_json(
    http: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
) -> Result<(u16, Option<u64>, String), LlmError> {
    let resp = http
        .post(url)
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

async fn post_chat(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    body: &Value,
) -> Result<(u16, Option<u64>, String), LlmError> {
    post_json(
        http,
        &format!("{base_url}/v1/chat/completions"),
        api_key,
        body,
    )
    .await
}

/// Models whose chat-completions endpoint refused function tools, so every call
/// goes to `/v1/responses` instead (see [`crate::openai_responses`]). Keyed like
/// the other refusal sets, by `base_url` and model.
fn responses_models() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Model-id prefixes routed to `/v1/responses` from the first call.
///
/// ADR (OPENAI-RESPONSES, 2026-10-05): route `gpt-6*` and `o5*` proactively
/// rather than waiting for chat-completions to refuse. `gpt-6.1-sol` refuses
/// function tools on chat-completions whatever `reasoning_effort` says (it
/// rejects `"none"` too), and the agent loop always sends tools, so for this
/// family the refusal is certain and learning it costs one wasted round trip per
/// model per process. Rejected alternative: learn-only routing, which remains
/// the fallback for every model not on this list. Only the OpenAI flavour is
/// routed; vLLM serves chat-completions alone.
const RESPONSES_PREFIXES: &[&str] = &["gpt-6", "o5"];

fn routes_to_responses(key: &str, model: &str) -> bool {
    let lower = model.to_lowercase();
    RESPONSES_PREFIXES.iter().any(|p| lower.starts_with(p))
        || responses_models()
            .lock()
            .map(|set| set.contains(key))
            .unwrap_or(false)
}

/// Remove the API key from every string an error carries.
///
/// Providers echo keys back in error bodies (OpenAI's 401 names the key it was
/// given). Same rule as the API crate's `routes::llm::scrub`: a string shorter
/// than eight characters is not a hosted-provider key, and scrubbing it would
/// mangle ordinary words.
pub(crate) fn scrub_error(err: LlmError, api_key: &str) -> LlmError {
    let key = api_key.trim();
    if key.len() < 8 {
        return err;
    }
    let s = |t: String| t.replace(key, "[redacted]");
    match err {
        LlmError::Auth(t) => LlmError::Auth(s(t)),
        LlmError::ContextTooLarge(t) => LlmError::ContextTooLarge(s(t)),
        LlmError::ToolsUnsupported(t) => LlmError::ToolsUnsupported(s(t)),
        LlmError::InvalidRequest(t) => LlmError::InvalidRequest(s(t)),
        LlmError::Network(t) => LlmError::Network(s(t)),
        LlmError::Deserialize(t) => LlmError::Deserialize(s(t)),
        LlmError::Provider { status, body } => LlmError::Provider {
            status,
            body: s(body),
        },
        other @ (LlmError::RateLimited { .. } | LlmError::Overloaded) => other,
    }
}

pub async fn chat(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    req: &ChatRequest,
    flavor: Flavor,
) -> Result<ChatResponse, LlmError> {
    let key = refusal_key(base_url, &req.model);
    let result = if flavor == Flavor::OpenAi && routes_to_responses(&key, &req.model) {
        crate::openai_responses::chat(http, base_url, api_key, req).await
    } else {
        match chat_completions(http, base_url, api_key, req, flavor).await {
            Ok(resp) => Ok(resp),
            Err(Completions::RefusedTools) => {
                // Learned: this model calls tools only on the Responses API.
                if let Ok(mut set) = responses_models().lock() {
                    set.insert(key);
                }
                crate::openai_responses::chat(http, base_url, api_key, req).await
            }
            Err(Completions::Failed(e)) => Err(e),
        }
    };
    result.map_err(|e| scrub_error(e, api_key))
}

/// Why a chat-completions call did not produce an answer.
enum Completions {
    /// The OpenAI flavour refused function tools alongside reasoning; the caller
    /// moves the model to `/v1/responses`.
    RefusedTools,
    Failed(LlmError),
}

impl From<LlmError> for Completions {
    fn from(e: LlmError) -> Self {
        Completions::Failed(e)
    }
}

async fn chat_completions(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    req: &ChatRequest,
    flavor: Flavor,
) -> Result<ChatResponse, Completions> {
    let key = refusal_key(base_url, &req.model);
    let mut drop_temperature = temperature_refusers()
        .lock()
        .map(|set| set.contains(&key))
        .unwrap_or(false);
    let mut no_reasoning = reasoning_tool_refusers()
        .lock()
        .map(|set| set.contains(&key))
        .unwrap_or(false);
    let shaped = |completion_tokens: bool, drop_temperature: bool, no_reasoning: bool| {
        let mut body = build_body(req, completion_tokens, flavor);
        if let Some(obj) = body.as_object_mut() {
            if drop_temperature {
                obj.remove("temperature");
            }
            if no_reasoning {
                obj.insert("reasoning_effort".into(), json!("none"));
            }
        }
        body
    };
    // vLLM takes `max_tokens`. Asking it for `max_completion_tokens` first would
    // spend a round trip to learn something already known.
    let mut completion_tokens = flavor == Flavor::OpenAi;
    let mut body = shaped(completion_tokens, drop_temperature, no_reasoning);
    let (mut status, mut retry_after, mut text) = post_chat(http, base_url, api_key, &body).await?;

    // Older models reject max_completion_tokens; retry once with max_tokens.
    if status == 400 && text.contains("max_completion_tokens") {
        completion_tokens = false;
        body = shaped(completion_tokens, drop_temperature, no_reasoning);
        (status, retry_after, text) = post_chat(http, base_url, api_key, &body).await?;
    }

    // Reasoning models take only the default temperature. Retry once without it,
    // and remember, so the next call does not pay for the same refusal.
    if status == 400 && !drop_temperature && req.temperature.is_some() && refuses_temperature(&text)
    {
        drop_temperature = true;
        if let Ok(mut set) = temperature_refusers().lock() {
            set.insert(key);
        }
        body = shaped(completion_tokens, drop_temperature, no_reasoning);
        (status, retry_after, text) = post_chat(http, base_url, api_key, &body).await?;
    }

    // Reasoning models that refuse tools while reasoning. On api.openai.com the
    // only working answer is the Responses API: `gpt-6.1-sol` refuses
    // `reasoning_effort: "none"` as well (2026-10-05). The retry with reasoning
    // off is kept only for the vLLM flavour, which has no Responses endpoint.
    if status == 400 && refuses_tools_with_reasoning(&text) {
        if flavor == Flavor::OpenAi {
            return Err(Completions::RefusedTools);
        }
        if !no_reasoning {
            no_reasoning = true;
            if let Ok(mut set) = reasoning_tool_refusers().lock() {
                set.insert(refusal_key(base_url, &req.model));
            }
            body = shaped(completion_tokens, drop_temperature, no_reasoning);
            (status, retry_after, text) = post_chat(http, base_url, api_key, &body).await?;
        }
    }

    if status == 400 {
        if let Some(err) = classify_openai_400(&text) {
            return Err(err.into());
        }
    }
    if !(200..300).contains(&status) {
        return Err(classify_status(status, &text, retry_after).into());
    }

    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| LlmError::Deserialize(e.to_string()))?;
    Ok(parse_response(parsed)?)
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

    /// The title generator's schema leaves `additionalProperties` open, and
    /// api.openai.com answers `strict: true` on such a schema with a 400, so every
    /// OpenAI-keyed conversation fell back to an untitled row.
    #[test]
    fn strict_is_claimed_only_for_a_schema_openai_accepts_strictly() {
        let open = serde_json::json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"]
        });
        assert!(!strict_compatible(&open));
        let closed = serde_json::json!({
            "type": "object",
            "properties": {"title": {"type": "string"}},
            "required": ["title"],
            "additionalProperties": false
        });
        assert!(strict_compatible(&closed));
        let partly_required = serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}, "b": {"type": "string"}},
            "required": ["a"],
            "additionalProperties": false
        });
        assert!(!strict_compatible(&partly_required));

        let mut r = req_with_tools();
        r.schema = Some(open);
        assert_eq!(
            build_body(&r, true, Flavor::OpenAi)["response_format"]["json_schema"]["strict"],
            false
        );
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
