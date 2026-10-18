//! OpenAI's newer reasoning models call tools only on `/v1/responses`
//! (OPENAI-RESPONSES track, 2026-10-05).
//!
//! Seen live: `gpt-6.1-sol` answers a chat-completions request carrying tools with
//! a 400 saying to use `/v1/responses` or set `reasoning_effort` to "none", and
//! then refuses "none" too. Before this fix the agent loop could not run a single
//! step on that model. The adapter now moves a refusing model to the Responses API
//! and remembers it; `gpt-6*` / `o5*` go there from the first call.
//!
//! The provider is a raw TCP server speaking just enough HTTP/1.1 for reqwest
//! (same pattern as `openai_reasoning_models.rs`).

use std::sync::{Arc, Mutex};

use llm::{ChatRequest, LlmClient, LlmError, Message, Provider, StopReason, ToolCall, ToolDef};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Every (path, body) the fake saw, in order.
type Seen = Arc<Mutex<Vec<(String, Value)>>>;
type Behaviour = fn(&str, &Value) -> (&'static str, Value);

const KEY: &str = "sk-proj-secret-fixture-123456";

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<(String, Value)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(end) = text.find("\r\n\r\n") {
            let path = text
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let len = text[..end]
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                let body = serde_json::from_slice(&buf[end + 4..end + 4 + len]).ok()?;
                return Some((path, body));
            }
        }
    }
}

async fn fake(behaviour: Behaviour) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let Some((path, body)) = read_request(&mut sock).await else {
                    return;
                };
                log.lock().unwrap().push((path.clone(), body.clone()));
                let (status, reply) = behaviour(&path, &body);
                let text = reply.to_string();
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (base, seen)
}

/// Like `gpt-6.1-sol`: chat-completions refuses tools; Responses answers with one
/// function call (behind a reasoning item, as the real API returns it).
fn sol(path: &str, _body: &Value) -> (&'static str, Value) {
    if path == "/v1/chat/completions" {
        return (
            "400 Bad Request",
            json!({"error": {
                "message": "Function tools with reasoning_effort are not supported for gpt-6.1-sol in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.",
                "type": "invalid_request_error"
            }}),
        );
    }
    (
        "200 OK",
        json!({
            "id": "resp_1",
            "object": "response",
            "status": "completed",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": []},
                {"type": "function_call", "id": "fc_item_1", "call_id": "call_abc",
                 "name": "lookup", "arguments": "{\"q\":\"btc\"}", "status": "completed"}
            ],
            "usage": {"input_tokens": 120, "output_tokens": 9,
                      "input_tokens_details": {"cached_tokens": 100}}
        }),
    )
}

fn request(model: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        system: Some("Answer with a tool call.".into()),
        messages: vec![Message::User {
            content: "price of btc?".into(),
        }],
        tools: vec![ToolDef {
            name: "lookup".into(),
            description: "look things up".into(),
            input_schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
        }],
        max_tokens: 256,
        temperature: Some(0.0),
        ..ChatRequest::default()
    }
}

/// (1) A model chat-completions refuses tools for is retried on Responses, and the
/// second call goes straight there.
#[tokio::test]
async fn a_model_refusing_tools_moves_to_responses_and_is_remembered() {
    let (base, seen) = fake(sol).await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    // Not a proactively routed prefix: this model must be *learned*.
    let req = request("gpt-5.9-sol-learned");

    let first = client.chat(&req).await.expect("answered on /v1/responses");
    assert_eq!(first.stop_reason, StopReason::ToolUse);
    assert_eq!(first.tool_calls.len(), 1);
    assert_eq!(first.tool_calls[0].id, "call_abc", "the call_id, not the item id");
    assert_eq!(first.tool_calls[0].name, "lookup");
    assert_eq!(first.tool_calls[0].arguments["q"], "btc");
    assert_eq!(first.usage.input_tokens, 20);
    assert_eq!(first.usage.cache_read_input_tokens, 100);
    assert_eq!(first.usage.prompt_tokens(), 120);
    assert_eq!(first.usage.output_tokens, 9);
    {
        let seen = seen.lock().unwrap();
        let paths: Vec<&str> = seen.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["/v1/chat/completions", "/v1/responses"]);
        let body = &seen[1].1;
        assert_eq!(body["model"], "gpt-5.9-sol-learned");
        assert_eq!(body["instructions"], "Answer with a tool call.");
        assert_eq!(body["max_output_tokens"], 256);
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["store"], false);
        assert!(body.get("temperature").is_none(), "reasoning models refuse it");
        assert!(body.get("messages").is_none());
        // Responses' flat function-tool shape.
        assert_eq!(
            body["tools"][0],
            json!({"type": "function", "name": "lookup", "description": "look things up",
                   "parameters": {"type": "object", "properties": {"q": {"type": "string"}}}})
        );
        assert_eq!(body["input"][0], json!({"role": "user", "content": "price of btc?"}));
    }

    client.chat(&req).await.expect("second call");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "no second refusal paid");
    assert_eq!(seen[2].0, "/v1/responses");
}

/// `gpt-6*` goes to Responses from the first call (ADR in `openai.rs`).
#[tokio::test]
async fn the_gpt_6_family_goes_to_responses_from_the_first_call() {
    let (base, seen) = fake(sol).await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    client
        .chat(&request("gpt-6.1-sol"))
        .await
        .expect("answered");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "/v1/responses");
}

/// vLLM serves chat-completions only and is never routed to Responses.
#[tokio::test]
async fn vllm_is_never_routed_to_responses() {
    let (base, seen) = fake(|_, _| {
        (
            "200 OK",
            json!({"choices": [{"finish_reason": "stop",
                    "message": {"content": "hi"}}]}),
        )
    })
    .await;
    let client = LlmClient::new(Provider::Vllm, None, Some(base));
    client.chat(&request("gpt-6-local")).await.expect("answered");
    assert_eq!(seen.lock().unwrap()[0].0, "/v1/chat/completions");
}

/// (2) A transcript with a prior tool call and its result: the `call_id` the model
/// returned is the one sent back on both the `function_call` and its output.
#[tokio::test]
async fn a_tool_call_round_trips_with_its_call_id() {
    let (base, seen) = fake(sol).await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    let mut req = request("gpt-6-roundtrip");
    let first = client.chat(&req).await.expect("first step");
    let call: ToolCall = first.tool_calls[0].clone();

    // What the agent loop appends after executing the call.
    req.messages.push(Message::Assistant {
        content: Some("Looking it up.".into()),
        tool_calls: vec![call.clone()],
    });
    req.messages.push(Message::ToolResult {
        tool_call_id: call.id.clone(),
        name: call.name.clone(),
        content: "64000".into(),
        is_error: false,
    });
    req.messages.push(Message::ToolResult {
        tool_call_id: "call_failed".into(),
        name: "lookup".into(),
        content: "no such symbol".into(),
        is_error: true,
    });
    client.chat(&req).await.expect("second step");

    let seen = seen.lock().unwrap();
    let input = seen[1].1["input"].as_array().expect("input").clone();
    assert_eq!(input[0], json!({"role": "user", "content": "price of btc?"}));
    assert_eq!(
        input[1],
        json!({"role": "assistant", "content": "Looking it up."})
    );
    assert_eq!(input[2]["type"], "function_call");
    assert_eq!(input[2]["call_id"], "call_abc");
    assert_eq!(input[2]["name"], "lookup");
    // Arguments are a JSON string on the wire.
    let args: Value = serde_json::from_str(input[2]["arguments"].as_str().expect("string"))
        .expect("json");
    assert_eq!(args, json!({"q": "btc"}));
    assert_eq!(
        input[3],
        json!({"type": "function_call_output", "call_id": "call_abc", "output": "64000"})
    );
    assert_eq!(input[4]["output"], "ERROR: no such symbol");
    assert_eq!(input.len(), 5);
}

/// (3) A text-only answer parses to EndTurn with usage; reasoning items are ignored.
#[tokio::test]
async fn a_text_answer_is_end_turn_with_usage() {
    let (base, _seen) = fake(|_, _| {
        (
            "200 OK",
            json!({
                "status": "completed",
                "output": [
                    {"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "thinking"}]},
                    {"type": "message", "role": "assistant", "content": [
                        {"type": "output_text", "text": "BTC is ", "annotations": []},
                        {"type": "output_text", "text": "64000.", "annotations": []}
                    ]}
                ],
                "usage": {"input_tokens": 50, "output_tokens": 7}
            }),
        )
    })
    .await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    let resp = client.chat(&request("o5-mini-text")).await.expect("answered");
    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    assert_eq!(resp.content.as_deref(), Some("BTC is 64000."));
    assert!(resp.tool_calls.is_empty());
    assert_eq!(resp.usage.input_tokens, 50);
    assert_eq!(resp.usage.output_tokens, 7);
    assert_eq!(resp.usage.cache_read_input_tokens, 0);
}

/// (4) The key never appears in a returned error, whatever the server echoes.
#[tokio::test]
async fn the_key_never_appears_in_an_error() {
    let (base, _seen) = fake(|_, _| {
        (
            "401 Unauthorized",
            json!({"error": {"message": format!("Incorrect API key provided: {KEY}."),
                              "code": "invalid_api_key"}}),
        )
    })
    .await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    let err = client
        .chat(&request("gpt-6-auth"))
        .await
        .expect_err("401");
    assert!(matches!(err, LlmError::Auth(_)), "{err:?}");
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains(KEY), "key leaked: {shown}");
    assert!(shown.contains("[redacted]"));
}

/// Errors on Responses go through the same classification as chat-completions.
#[tokio::test]
async fn responses_errors_are_classified() {
    let (base, _seen) = fake(|_, _| {
        (
            "400 Bad Request",
            json!({"error": {"message": "Your input exceeds the context window of this model.",
                              "code": "context_length_exceeded"}}),
        )
    })
    .await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    let err = client.chat(&request("gpt-6-ctx")).await.expect_err("400");
    assert!(matches!(err, LlmError::ContextTooLarge(_)), "{err:?}");

    let (base, _seen) = fake(|_, _| {
        (
            "429 Too Many Requests\r\nretry-after: 7",
            json!({"error": {"message": "slow down"}}),
        )
    })
    .await;
    let client = LlmClient::new(Provider::OpenAi, Some(KEY.into()), Some(base));
    let err = client.chat(&request("gpt-6-rate")).await.expect_err("429");
    assert!(
        matches!(
            err,
            LlmError::RateLimited {
                retry_after_secs: Some(7)
            }
        ),
        "{err:?}"
    );
}
