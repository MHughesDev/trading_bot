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

/// The beta that lets a cache breakpoint live longer than five minutes.
///
/// Without it the `ttl` below is ignored and the breakpoints fall back to 5m.
const EXTENDED_CACHE_BETA: &str = "extended-cache-ttl-2025-04-11";

/// How long a cache breakpoint lives (AGENT-005 §6.1; Set S task S-2.7).
///
/// One hour, not the five-minute default. A research session runs for hours with a
/// stable charter and a stable tool slice, and a 5m TTL re-pays for the same prefix
/// over and over — the breakpoints S-1.8 places are only worth placing if they
/// outlive the gap between one step and the next, and a step that waits on a tool
/// can easily take longer than five minutes.
///
/// The same value `llm_proxy::rewrite_cache_ttl` forces on the container's requests.
/// It is set here rather than only there so **every** caller gets it: the proxy only
/// sees what the CLI sends, and the internal drivers do not go through it.
const CACHE_TTL: &str = "1h";

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

/// The marker that closes a cache prefix.
///
/// Anthropic caches the prompt *up to and including* the block carrying this, so a
/// breakpoint is a statement about everything before it, not about the block it
/// sits on. That is why the three below are placed at the end of each layer rather
/// than at its start.
fn cache_control() -> Value {
    json!({ "type": "ephemeral", "ttl": CACHE_TTL })
}

/// Three breakpoints, in prefix order (AGENT-005 §6.1, task S-1.8).
///
/// The layers are ordered by how often they change, longest-lived first, because a
/// prefix is only reusable if everything before it is byte-identical:
///
/// 1. **tools** — the catalogue slice for this step. Changes when routing changes.
/// 2. **system** — the charter. Byte-stable for the life of the run.
/// 3. **the last content block of the last message** — closes the conversation so
///    far, so the next turn reads everything up to here from the cache.
///
/// A segment under the provider's minimum cacheable length is ignored by the API
/// rather than rejected, so marking a small prompt is safe and costs nothing. The
/// failure this avoids is the opposite one: no breakpoints at all, and a 40,000-token
/// prompt re-billed in full on every one of thirty iterations.
fn build_body(req: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("max_tokens".into(), json!(req.max_tokens));

    let mut messages = build_messages(req);
    // Breakpoint 3, innermost. The *last content block* of the *last* message, so a
    // plain-string message has to become a block array to carry it — and only that
    // one does. An earlier user message stays a plain string, which is what
    // `cache_breakpoints_close_each_prefix_layer` asserts: marking every message
    // would create a separate prefix per turn and cache nothing usefully.
    if let Some(last) = messages.last_mut() {
        let content = last.get("content").cloned().unwrap_or(Value::Null);
        let mut blocks: Vec<Value> = match content {
            Value::Array(a) => a,
            Value::String(t) => vec![json!({ "type": "text", "text": t })],
            _ => Vec::new(),
        };
        if let Some(block) = blocks.last_mut() {
            if let Some(obj) = block.as_object_mut() {
                obj.insert("cache_control".into(), cache_control());
            }
        }
        if !blocks.is_empty() {
            if let Some(obj) = last.as_object_mut() {
                obj.insert("content".into(), Value::Array(blocks));
            }
        }
    }
    body.insert("messages".into(), Value::Array(messages));

    if let Some(system) = &req.system {
        // Breakpoint 2. A text block rather than a bare string, because a string
        // has nowhere to put the marker.
        body.insert(
            "system".into(),
            json!([{ "type": "text", "text": system, "cache_control": cache_control() }]),
        );
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
        // Breakpoint 1, outermost: the LAST tool only. The marker closes the whole
        // tools array, so marking every entry would be three dozen prefixes where
        // one is meant.
        let mut tools = tools;
        if let Some(last) = tools.last_mut() {
            if let Some(obj) = last.as_object_mut() {
                obj.insert("cache_control".into(), cache_control());
            }
        }
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

    let usage = Usage {
        input_tokens: body
            .pointer("/usage/input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: body
            .pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        // Absent on a response that used no cache, which is why both default to
        // zero rather than being required. Zero here is honest: nothing was read
        // from the cache and nothing was written to it.
        cache_read_input_tokens: body
            .pointer("/usage/cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cache_creation_input_tokens: body
            .pointer("/usage/cache_creation_input_tokens")
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
        .header("anthropic-beta", EXTENDED_CACHE_BETA)
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
        .header("anthropic-beta", EXTENDED_CACHE_BETA)
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
    use crate::types::ToolDef;

    /// The three breakpoints, and — as importantly — where there are none.
    ///
    /// A prefix is only reusable if everything before it is byte-identical, so the
    /// marker has to sit at the END of each layer and nowhere else. Marking every
    /// tool would make three dozen prefixes where one is meant; marking every
    /// message would make a fresh prefix per turn and cache nothing.
    #[test]
    fn cache_breakpoints_close_each_prefix_layer() {
        let req = ChatRequest {
            model: "claude-test".into(),
            system: Some("the charter".into()),
            messages: vec![
                Message::User {
                    content: "first".into(),
                },
                Message::User {
                    content: "second".into(),
                },
            ],
            tools: vec![
                ToolDef {
                    name: "a".into(),
                    description: "first".into(),
                    input_schema: json!({"type": "object"}),
                },
                ToolDef {
                    name: "b".into(),
                    description: "last".into(),
                    input_schema: json!({"type": "object"}),
                },
            ],
            ..ChatRequest::default()
        };
        let body = build_body(&req);

        // 1. Tools: the last one only.
        let tools = body["tools"].as_array().expect("tools");
        assert!(
            tools[0].get("cache_control").is_none(),
            "not the first tool"
        );
        assert_eq!(
            tools[1]["cache_control"]["type"], "ephemeral",
            "the last tool closes the layer"
        );

        // 2. System: a text block, because a bare string has nowhere to put it.
        let system = body["system"].as_array().expect("system is a block array");
        assert_eq!(system[0]["text"], "the charter");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");

        // 3. The last content block of the LAST message, and only that one.
        let messages = body["messages"].as_array().expect("messages");
        assert!(
            messages[0]["content"].is_string(),
            "an earlier message stays a plain string: {}",
            messages[0]["content"]
        );
        let last = messages.last().expect("a last message");
        let blocks = last["content"]
            .as_array()
            .expect("the last message carries blocks");
        assert_eq!(blocks.last().unwrap()["cache_control"]["type"], "ephemeral");
    }

    /// Every breakpoint carries the one-hour TTL, not the five-minute default.
    ///
    /// A research session runs for hours with a stable charter and tool slice, and a
    /// 5m TTL re-pays for the same prefix over and over — the breakpoints S-1.8
    /// places are only worth placing if they outlive the gap between one step and the
    /// next, and a step that waits on a tool can easily exceed five minutes.
    #[test]
    fn every_breakpoint_lives_an_hour() {
        let req = ChatRequest {
            model: "claude-test".into(),
            system: Some("the charter".into()),
            messages: vec![Message::User {
                content: "go".into(),
            }],
            tools: vec![ToolDef {
                name: "t".into(),
                description: "d".into(),
                input_schema: json!({"type": "object"}),
            }],
            ..ChatRequest::default()
        };
        let body = build_body(&req);
        for place in [
            body["tools"][0]["cache_control"].clone(),
            body["system"][0]["cache_control"].clone(),
            body["messages"][0]["content"][0]["cache_control"].clone(),
        ] {
            assert_eq!(place["type"], "ephemeral");
            assert_eq!(
                place["ttl"], "1h",
                "a five-minute breakpoint re-pays for the same prefix every step"
            );
        }
    }

    /// A request with no tools and no system still produces a valid body: the
    /// breakpoints are placed where the layers exist, not asserted into being.
    #[test]
    fn a_request_with_no_tools_and_no_system_is_still_well_formed() {
        let req = ChatRequest {
            model: "claude-test".into(),
            messages: vec![Message::User {
                content: "hi".into(),
            }],
            ..ChatRequest::default()
        };
        let body = build_body(&req);
        assert!(body.get("tools").is_none());
        assert!(body.get("system").is_none());
        let blocks = body["messages"][0]["content"].as_array().expect("blocks");
        assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
    }

    /// `input_tokens` is what was BILLED as fresh, not how big the prompt was.
    /// Reading the first as the second is how a budget comes to believe it has
    /// thirty-nine thousand tokens of headroom it does not have.
    #[test]
    fn a_cached_response_reports_the_whole_prompt_not_only_the_fresh_part() {
        let body = json!({
            "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "cache_read_input_tokens": 900,
                "cache_creation_input_tokens": 90
            }
        });
        let resp = parse_response(body).expect("parses");
        assert_eq!(resp.usage.input_tokens, 10, "fresh tokens, as billed");
        assert_eq!(
            resp.usage.prompt_tokens(),
            1_000,
            "the prompt was a thousand tokens"
        );
        let hit = resp.usage.cache_hit_fraction().expect("there was a prompt");
        assert!((hit - 0.9).abs() < 1e-9, "90% served from cache, got {hit}");
    }

    /// And a response that used no cache reports the prompt unchanged — the two
    /// counters default to zero rather than being required, because they are
    /// absent on a call that never touched the cache.
    #[test]
    fn an_uncached_response_reports_the_prompt_as_its_fresh_tokens() {
        let body = json!({
            "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 1_000, "output_tokens": 5 }
        });
        let resp = parse_response(body).expect("parses");
        assert_eq!(resp.usage.prompt_tokens(), 1_000);
        assert_eq!(resp.usage.cache_hit_fraction(), Some(0.0));
    }

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
        // System prompt is top-level, not a message. A block array rather than a
        // bare string since S-1.8: a string has nowhere to carry the cache marker.
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
