//! MCP server process — JSON-RPC 2.0 over stdio.
//!
//! Reads newline-delimited JSON-RPC 2.0 requests from stdin and writes
//! responses to stdout. Implements the subset of the MCP protocol needed
//! to expose the nine strategy tools.
//!
//! Standard MCP methods handled:
//! - `initialize` — capability negotiation
//! - `tools/list`  — advertise available tools
//! - `tools/call`  — dispatch to a tool handler

use std::io::Write;

use tokio::io::{AsyncBufReadExt, BufReader};

use mcp_server_lib::{dispatch_tool, tool_definitions, McpContext};

#[tokio::main]
async fn main() {
    let ctx = McpContext::new().await;
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                let response = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("parse error: {e}") }
                });
                writeln!(out, "{}", serde_json::to_string(&response).unwrap()).ok();
                out.flush().ok();
                continue;
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

        let response = match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
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

        writeln!(out, "{}", serde_json::to_string(&response).unwrap()).ok();
        out.flush().ok();
    }
}
