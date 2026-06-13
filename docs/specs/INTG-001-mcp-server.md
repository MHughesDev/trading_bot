# INTG-001: MCP Server

**Status:** Implemented
**Version:** 2.0 (Set F)
**ADR(s):** ADR-0010
**Success Conditions:** SC-2

## 1. Purpose

Defines the MCP (Model Context Protocol) server: a fully capable AI agent platform that enables
an agent (e.g. Claude) to construct strategies incrementally, run backtests, and configure
automations. All operations route through the same validator, risk gate, and runtime as the
visual builder and JSON API. The MCP server adds no privileged path.

## 2. Transport

**Streamable HTTP (MCP spec 2025-03-26).** The server exposes:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/mcp` | POST | JSON-RPC 2.0 request/response. Returns `text/event-stream` when client sends `Accept: text/event-stream`. |
| `/health` | GET | Liveness check — returns `{"status":"ok"}` with HTTP 200. |

**Bind address:** `127.0.0.1:3002` by default. Configurable via `MCP_PORT` env var.
The server binds to loopback only until Set E Phase 1 (cookie-session auth) lands.

**SSE upgrade:** When the client sends `Accept: text/event-stream`, the single response
is encoded as:
```
event: message
data: <JSON-RPC response JSON>

```

## 3. McpContext Fields

| Field | Type | Description |
|-------|------|-------------|
| `strategy_store` | `Arc<Mutex<HashMap<Uuid, StrategyDefinition>>>` | In-process strategy library |
| `instance_manager` | `Arc<Mutex<InstanceManager>>` | Running strategy instances |
| `pg` | `Option<PgPool>` | Postgres pool; `None` when `DATABASE_URL` unset |
| `draft_store` | `Arc<Mutex<HashMap<Uuid, StrategyDraft>>>` | In-memory draft state for builder |
| `backtest_manager` | `Option<Arc<BacktestManager>>` | Backtest engine; `None` when `CLICKHOUSE_URL` or `DATABASE_URL` unset |

## 4. Tool Surface

### 4.1 Discovery Tools

| Tool | Required Params | Returns |
|------|----------------|---------|
| `list_lanes` | — | `{ lanes: [{ lane }] }` |
| `list_instruments` | — | `{ instruments: [...] }` |

Optional: `list_instruments` accepts `asset_class` filter.

### 4.2 Authoring Tools

| Tool | Required Params | Returns |
|------|----------------|---------|
| `validate_strategy` | `definition_json` | `{ valid, errors }` |
| `create_strategy` | `definition_json` | `{ strategy_id, store_id }` or `{ error, errors }` |

### 4.3 Lifecycle Tools

| Tool | Required Params | Returns |
|------|----------------|---------|
| `list_strategies` | — | `{ strategies: [{ store_id, strategy_id }] }` |
| `apply_strategy` | `store_id`, `user_id`, `instrument_id` | `{ store_id, user_id, instrument_id, status }` or error |
| `stop_strategy` | `user_id`, `instrument_id` | `{ user_id, instrument_id, stopped }` |

### 4.4 Strategy Builder Tools

An agent creates a draft, adds components incrementally, then finalizes. Drafts are
in-memory only (lost on restart). A `draft_id` UUID is the key passed to all builder tools.

| Tool | Required Params | Optional Params | Effect |
|------|----------------|-----------------|--------|
| `new_strategy_draft` | — | — | Returns `{ draft_id }` |
| `discard_draft` | `draft_id` | — | Returns `{ discarded: bool }` |
| `set_strategy_meta` | `draft_id`, `strategy_id`, `asset_class` | `min_trust_tier` | Sets top-level fields |
| `add_strategy_input` | `draft_id`, `lane` | `instrument`, `features` | Appends `InputDeclaration` |
| `add_condition_node` | `draft_id`, `node_id`, `expr` | — | Appends Condition node; duplicate `node_id` → `{ error: "duplicate_node_id" }` |
| `add_signal_node` | `draft_id`, `node_id`, `when`, `emit` | — | Appends Signal node |
| `add_strategy_action` | `draft_id`, `on_signal`, `side`, `size_mode`, `size` | — | Appends PlaceOrder action |
| `set_risk_overrides` | `draft_id` | `max_position`, `max_order_rate_per_minute`, `max_order_rate_per_second` | Overwrites risk overrides |
| `get_draft_summary` | `draft_id` | — | Returns current draft definition JSON (no mutation) |
| `finalize_strategy` | `draft_id` | — | Validate → persist → return `{ store_id, strategy_id, valid: true }` or `{ valid: false, errors }` |

**Mutation tool response shape (success):**
```json
{ "draft_id": "<uuid>", "strategy_id": "ema_cross_v1", "inputs_count": 2, "nodes_count": 2, "actions_count": 1 }
```

**Unknown draft_id response:**
```json
{ "error": "draft_not_found", "draft_id": "<uuid>" }
```

### 4.5 Backtest Tools

Requires both `CLICKHOUSE_URL` and `DATABASE_URL` to be set. Otherwise all three tools
return `{ "error": "service_unavailable", "reason": "backtest service not configured" }`.

| Tool | Required Params | Optional Params | Returns |
|------|----------------|-----------------|---------|
| `list_backtests` | — | — | `{ backtests: [...] }` (empty if no runs) |
| `get_backtest` | `backtest_id` | — | Full `BacktestSnapshot` or `{ error: "not_found" }` |
| `create_backtest` | `store_id`, `instrument_id`, `asset_class`, `timeframe`, `start`, `end` | `name`, `initial_balance`, `quote_currency`, `auto_collect` | `{ backtest_id, status: "Queued" }` |

Valid timeframes: `"1s"`, `"1m"`, `"5m"`, `"15m"`, `"1h"`, `"4h"`, `"1d"`.

`create_backtest` error cases:
- `strategy_not_found` — `store_id` not in strategy store
- `invalid_timeframe` — includes `valid_values` array
- `invalid_date_range` — `end ≤ start`
- `service_unavailable` — BacktestManager not configured

### 4.6 Automation Tools

Requires `DATABASE_URL`. All write tools return `{ error: "service_unavailable" }` if `pg` is `None`.

| Tool | Required Params | Optional Params | Returns |
|------|----------------|-----------------|---------|
| `list_automations` | — | — | `{ automations: [...] }` (empty list on error or no pg) |
| `create_automation` | `execution_strategy_id`, `instrument_id`, `asset_class`, `account_mode` | `armed`, `time_window_start`, `time_window_end`, `time_window_tz` | `{ automation_id, armed, kind }` or error |
| `arm_automation` | `automation_id` | — | `{ automation_id, armed: true }` or `{ error: "not_found" }` |
| `disarm_automation` | `automation_id` | — | `{ automation_id, armed: false }` or `{ error: "not_found" }` |

**Live automation gate:** `create_automation` with `account_mode: "live"` returns
`{ error: "live_automations_disabled" }` unless `MCP_ALLOW_LIVE_AUTOMATIONS=true` (or `=1`) is set.
Paper mode is unrestricted. Default: off.

## 5. Guardrails

**No order placement tool.** There is no `place_order` MCP tool. Strategies emit order intents
only through the runtime, which passes them through `RiskGate::check()`.

**Validation is mandatory before persist.** `create_strategy` and `finalize_strategy` both run
the shared `strategy_validator::validate()` before inserting into the strategy store.

**Risk overrides tighten only.** The shared validator enforces this; MCP cannot sidestep it.

**Trust tier respected.** Strategies carry `min_trust_tier`; the risk gate enforces it at order time.

**Loopback bind.** The HTTP server binds to `127.0.0.1` only. This guard is removed only after
Set E Phase 1 (cookie-session auth) and TLS are in place.

**Live automation gate.** `create_automation` with `account_mode: "live"` is blocked by default.
Set `MCP_ALLOW_LIVE_AUTOMATIONS=true` to enable. Paper mode is unrestricted.

**DEV_USER placeholder.** Backtest and automation tools use `Uuid::nil()` as the user identity
until Set E Phase 1 auth lands — matching the REST API's current state.

## 6. Scope & Non-Goals

**In scope (v2.0):**
- Streamable HTTP transport (replaces stdio)
- Step-by-step strategy builder (10 tools)
- Backtest trigger/poll/read (3 tools)
- Automation assign/arm/disarm (4 tools)
- Health endpoint
- Live automation gate
- Integration tests for all three agent workflows

**Not in scope:**
- Pipeline automations (Set E / future scope)
- Authentication of MCP callers (Set E Phase 1)
- TLS / public bind (requires auth first)
- New node or action kinds (ADR-0007 frozen)

## 7. Dependencies

- DATA-004 — canonical strategy definition format (v1.0, frozen)
- FEAT-001 — strategy runtime
- COMP-002 — risk gate
- ADR-0014 — backtests via BacktestManager only

## 8. Acceptance Criteria

- [x] AC-1: `cargo build -p mcp-server` succeeds with no new warnings.
- [x] AC-2: `POST /mcp` with `tools/list` returns all 27 tool definitions.
- [x] AC-3: `GET /health` returns `{"status":"ok"}`.
- [x] AC-4: Server binds to `127.0.0.1` only.
- [x] AC-5: `finalize_strategy` on a valid draft removes the draft and returns `store_id`.
- [x] AC-6: `finalize_strategy` on an invalid draft returns `valid: false` with errors; draft intact.
- [x] AC-7: `create_automation` with `account_mode: "live"` (default env) returns `live_automations_disabled`.
- [x] AC-8: `cargo test -p mcp-server-lib` passes all workflow integration tests.
- [x] AC-9: No stdin/stdout references remain in `apps/mcp-server/src/main.rs`.
