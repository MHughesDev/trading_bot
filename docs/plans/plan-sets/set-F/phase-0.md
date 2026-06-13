# Phase 0 — HTTP Transport

**Completion: 0% (0 / 1 tasks)**

**Goal:** Replace the stdio JSON-RPC transport with Streamable HTTP (MCP spec
2025-03-26) so any MCP-compliant client can reach the server over the network.
This is the prerequisite for all Phase 1–4 work.

---

## Tasks

### ☐ F-0.1 Replace stdio with Streamable HTTP (MCP 2025-03-26) — M — **transport**

**Why.** The current server reads from stdin and writes to stdout
(`apps/mcp-server/src/main.rs`). This works for local CLI usage (e.g. Claude
Desktop with a spawned subprocess) but cannot be reached by remote clients,
CI pipelines, or web-based agents. Streamable HTTP (the current MCP spec,
2025-03-26) uses a single `POST /mcp` endpoint that accepts JSON-RPC 2.0
requests and optionally upgrades to Server-Sent Events for streaming. Axum is
already the HTTP layer in `apps/platform`; reuse it here.

**What to build:**
- Add `axum`, `tokio`, `tower-http` to `apps/mcp-server/Cargo.toml`.
- Implement a `POST /mcp` handler that:
  - Deserialises the JSON-RPC 2.0 request body.
  - Dispatches `initialize`, `tools/list`, `tools/call` (routes already
    exist in the current main.rs loop — lift them into async handler fns).
  - Returns a JSON-RPC 2.0 response for single-turn calls.
  - Returns an SSE stream (`text/event-stream`) when the client sends
    `Accept: text/event-stream`, encoding each response chunk as a `data:`
    event with `event: message`.
- Add a `GET /health` endpoint returning `200 OK` with `{"status":"ok"}`.
- Bind to `127.0.0.1:3002` by default; make port configurable via
  `MCP_PORT` env var. Do **not** bind to `0.0.0.0` until Set E Phase 1
  (session auth) is complete — loopback guard matches the one at
  `apps/platform/src/main.rs:42-48`.
- Remove the stdin/stdout loop from `main.rs`; the new entrypoint is just
  `axum::serve(listener, router).await`.
- `McpContext::new()` is already async and database-aware; call it once at
  startup and wrap in `Arc` for sharing across handlers.

**Files:**
- `apps/mcp-server/src/main.rs` — replace entirely
- `apps/mcp-server/Cargo.toml` — add axum, tower-http deps
- `crates/mcp-server/src/lib.rs` — `dispatch_tool` already async; ensure
  `McpContext` is `Clone + Send + Sync` (it is, via Arc fields)

**Acceptance criteria:**
- `cargo build -p mcp-server` succeeds with no new warnings.
- `curl -X POST http://127.0.0.1:3002/mcp -H 'Content-Type: application/json' \`
  `-d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'`
  returns the 7 existing tool definitions as a valid JSON-RPC 2.0 response.
- `curl -X POST .../mcp -H 'Accept: text/event-stream' ...` returns
  `Content-Type: text/event-stream` with `data:` prefixed JSON-RPC response.
- `GET /health` returns `{"status":"ok"}`.
- No stdin/stdout references remain in `apps/mcp-server/src/main.rs`.
- Loopback-only bind is enforced; attempting to set `MCP_PORT=80` on a
  non-loopback interface is either rejected or the listen address is forced
  to `127.0.0.1`.
