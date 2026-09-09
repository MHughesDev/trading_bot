//! MCP server process — JSON-RPC 2.0 over Streamable HTTP (MCP spec 2025-03-26).
//!
//! A thin, unprivileged front door (ADR-0010): every tool call is executed by
//! the platform's own authenticated HTTP API using the service token in
//! `PLATFORM_API_TOKEN`. This process holds no database access of its own.
//!
//! Binds to `127.0.0.1:3002` (or `MCP_PORT`) and exposes:
//!   POST /mcp  — JSON-RPC 2.0 request/response; when the client accepts
//!                `text/event-stream`, tool calls stream keep-alive/progress
//!                frames so long-running tools (`wait_for_backtest`) survive
//!                client and proxy idle timeouts
//!   GET  /health — liveness check

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use mcp_server_lib::{
    dispatch_tool, server_instructions, tool_definitions, ApiClient, McpContext, ProgressUpdate,
};

type SharedCtx = Arc<McpContext>;

/// Seconds between keep-alive frames while a tool call is in flight.
const KEEPALIVE_SECS: u64 = 10;

#[tokio::main]
async fn main() {
    observability::init("mcp-server");

    let api = match ApiClient::from_env() {
        Ok(api) => api,
        Err(e) => {
            eprintln!("mcp-server: {e}");
            std::process::exit(1);
        }
    };
    let ctx = Arc::new(McpContext::new(api));

    let port: u16 = std::env::var("MCP_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3002);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let app = Router::new()
        .route("/mcp", post(mcp_handler))
        .route("/health", get(health_handler))
        .with_state(ctx);

    tracing::info!("MCP HTTP server listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

fn sse_frame(payload: &serde_json::Value) -> Bytes {
    let json_str = serde_json::to_string(payload).unwrap_or_default();
    Bytes::from(format!("event: message\ndata: {json_str}\n\n"))
}

fn tool_call_response(id: &serde_json::Value, result: &serde_json::Value) -> serde_json::Value {
    let is_error = result.get("error").is_some();
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(result).unwrap_or_default()
            }],
            "isError": is_error
        }
    })
}

async fn mcp_handler(
    State(ctx): State<SharedCtx>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let request: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            let resp = serde_json::json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            });
            return (StatusCode::OK, Json(resp)).into_response();
        }
    };

    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = request
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    let wants_sse = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false);

    // Long-running tool calls stream keep-alive/progress frames; everything
    // else is answered immediately.
    if method == "tools/call" && wants_sse {
        return streaming_tool_call(ctx, id, params).await;
    }

    let result_value = match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "trading-bot-mcp", "version": "2.0" },
                "instructions": server_instructions()
            }
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tool_definitions() }
        }),
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tool_args = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            let result = dispatch_tool(&ctx, tool_name, &tool_args, None).await;
            tool_call_response(&id, &result)
        }
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {other}") }
        }),
    };

    if wants_sse {
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .body(Body::from(sse_frame(&result_value)))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    } else {
        (StatusCode::OK, Json(result_value)).into_response()
    }
}

/// Execute a tool call while streaming SSE frames so idle timeouts never fire:
/// spec `notifications/progress` frames when the client sent a progressToken
/// (these reset MCP client tool timeouts), bare comment frames otherwise.
async fn streaming_tool_call(
    ctx: SharedCtx,
    id: serde_json::Value,
    params: serde_json::Value,
) -> Response {
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tool_args = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let progress_token = params
        .get("_meta")
        .and_then(|m| m.get("progressToken"))
        .cloned();

    let (progress_tx, mut progress_rx) = mpsc::channel::<ProgressUpdate>(16);
    let mut dispatch = tokio::spawn({
        let ctx = ctx.clone();
        async move { dispatch_tool(&ctx, &tool_name, &tool_args, Some(progress_tx)).await }
    });

    let (bytes_tx, bytes_rx) = mpsc::channel::<Result<Bytes, Infallible>>(16);
    tokio::spawn(async move {
        let mut keepalive = tokio::time::interval(Duration::from_secs(KEEPALIVE_SECS));
        keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        keepalive.tick().await; // consume the immediate first tick

        loop {
            tokio::select! {
                res = &mut dispatch => {
                    let result = res.unwrap_or_else(|e| serde_json::json!({
                        "error": "tool_panicked",
                        "detail": e.to_string()
                    }));
                    let _ = bytes_tx.send(Ok(sse_frame(&tool_call_response(&id, &result)))).await;
                    break;
                }
                maybe_progress = progress_rx.recv() => {
                    let frame = match (&maybe_progress, &progress_token) {
                        (Some(p), Some(token)) => sse_frame(&serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/progress",
                            "params": {
                                "progressToken": token,
                                "progress": p.progress,
                                "total": 100.0,
                                "message": p.message
                            }
                        })),
                        (Some(_), None) => Bytes::from(": progress\n\n"),
                        (None, _) => {
                            // Sender dropped — the tool is done; emit its result.
                            let result = (&mut dispatch).await.unwrap_or_else(|e| serde_json::json!({
                                "error": "tool_panicked",
                                "detail": e.to_string()
                            }));
                            let _ = bytes_tx.send(Ok(sse_frame(&tool_call_response(&id, &result)))).await;
                            break;
                        }
                    };
                    if bytes_tx.send(Ok(frame)).await.is_err() {
                        break; // client went away
                    }
                }
                _ = keepalive.tick() => {
                    if bytes_tx.send(Ok(Bytes::from(": keepalive\n\n"))).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(ReceiverStream::new(bytes_rx)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
