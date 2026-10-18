//! OpenAI's reasoning models refuse a non-default `temperature` (PROVIDERS track,
//! 2026-10-05).
//!
//! Every frontier profile asks for `temperature_tool_calls: 0.0`, so before this
//! fix every agent step on a GPT-5-family or o-series model was a 400 and an
//! OpenAI-keyed user could not run a single turn. The adapter now retries once
//! without the parameter and remembers the model, so the refusal is paid once per
//! process rather than once per step.
//!
//! The provider is a raw TCP server speaking just enough HTTP/1.1 for reqwest,
//! so this needs no network and no dependency the crate does not already have.

use std::sync::{Arc, Mutex};

use llm::{ChatRequest, LlmClient, Message, Provider, StopReason, ToolDef};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Every request body the fake saw, in order.
type Seen = Arc<Mutex<Vec<Value>>>;

async fn read_request(sock: &mut tokio::net::TcpStream) -> Option<Value> {
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
                return serde_json::from_slice(&buf[end + 4..end + 4 + len]).ok();
            }
        }
    }
}

/// A chat-completions server that behaves like a reasoning model: any request
/// carrying `temperature` is a 400 naming it, and anything else answers with one
/// tool call.
async fn reasoning_model() -> (String, Seen) {
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
                let Some(body) = read_request(&mut sock).await else {
                    return;
                };
                log.lock().unwrap().push(body.clone());
                let (status, reply) = if body.get("temperature").is_some() {
                    (
                        "400 Bad Request",
                        json!({"error": {
                            "message": "Unsupported value: 'temperature' does not support 0.0 with this model. Only the default (1) value is supported.",
                            "type": "invalid_request_error",
                            "param": "temperature",
                            "code": "unsupported_value"
                        }}),
                    )
                } else {
                    (
                        "200 OK",
                        json!({
                            "choices": [{
                                "finish_reason": "tool_calls",
                                "message": {
                                    "content": null,
                                    "tool_calls": [{
                                        "id": "call_1",
                                        "type": "function",
                                        "function": {"name": "lookup", "arguments": "{\"q\":\"btc\"}"}
                                    }]
                                }
                            }],
                            "usage": {"prompt_tokens": 20, "completion_tokens": 4}
                        }),
                    )
                };
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

#[tokio::test]
async fn a_refused_temperature_is_retried_without_it_and_remembered() {
    let (base, seen) = reasoning_model().await;
    let client = LlmClient::new(Provider::OpenAi, Some("sk-test-fixture".into()), Some(base));

    let first = client
        .chat(&request("gpt-5-test"))
        .await
        .expect("the retry succeeds");
    assert_eq!(first.stop_reason, StopReason::ToolUse);
    assert_eq!(first.tool_calls[0].name, "lookup");
    assert_eq!(first.tool_calls[0].arguments["q"], "btc");
    assert_eq!(first.usage.input_tokens, 20);
    {
        let bodies = seen.lock().unwrap();
        assert_eq!(bodies.len(), 2, "one refusal, one retry");
        assert!(bodies[0].get("temperature").is_some());
        assert!(bodies[1].get("temperature").is_none());
        // The OpenAI flavour asks for completion tokens, which reasoning models need.
        assert_eq!(bodies[1]["max_completion_tokens"], 256);
    }

    // The second call on the same model does not pay for the refusal again.
    client
        .chat(&request("gpt-5-test"))
        .await
        .expect("second call");
    let bodies = seen.lock().unwrap();
    assert_eq!(bodies.len(), 3, "the model was remembered");
    assert!(bodies[2].get("temperature").is_none());
}

#[tokio::test]
async fn an_assistant_turn_with_nothing_in_it_is_sent_as_an_empty_string() {
    let (base, seen) = reasoning_model().await;
    let client = LlmClient::new(Provider::OpenAi, Some("sk-test-fixture".into()), Some(base));
    let mut req = request("gpt-5-empty-turn");
    req.temperature = None;
    req.messages.push(Message::Assistant {
        content: None,
        tool_calls: Vec::new(),
    });
    req.messages.push(Message::User {
        content: "go on".into(),
    });
    client.chat(&req).await.expect("accepted");
    let bodies = seen.lock().unwrap();
    // system, user, assistant, user
    assert_eq!(bodies[0]["messages"][2]["role"], "assistant");
    assert_eq!(bodies[0]["messages"][2]["content"], "");
}
