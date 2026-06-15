//! MCP server process — JSON-RPC 2.0 over Streamable HTTP (MCP spec 2025-03-26).
//!
//! Binds to `127.0.0.1:3002` (or `MCP_PORT`) and exposes:
//!   POST /mcp  — JSON-RPC 2.0 request/response; SSE stream when client sends
//!                `Accept: text/event-stream`
//!   GET  /health — liveness check

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

use mcp_server_lib::{dispatch_tool, tool_definitions, McpContext};

type SharedCtx = Arc<McpContext>;

#[tokio::main]
async fn main() {
    observability::init("mcp-server");

    let ctx = Arc::new(McpContext::new().await);

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

    let result_value = match method {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "trading-bot-mcp", "version": "1.0" }
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
            let result = dispatch_tool(&ctx, tool_name, &tool_args).await;
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result).unwrap_or_default() }]
                }
            })
        }
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {other}") }
        }),
    };

    let wants_sse = headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("text/event-stream"))
        .unwrap_or(false);

    if wants_sse {
        let json_str = serde_json::to_string(&result_value).unwrap_or_default();
        let sse_body = format!("event: message\ndata: {json_str}\n\n");
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .body(Body::from(sse_body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    } else {
        (StatusCode::OK, Json(result_value)).into_response()
    }
}
