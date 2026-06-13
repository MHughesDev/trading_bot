# Phase 2 — Backtest Tools

**Completion: 0% (0 / 3 tasks)**

**Goal:** Let an AI agent trigger a backtest against any persisted or
freshly-built strategy, poll its status, and read the full results — all via
MCP tool calls. Uses `backtest::BacktestManager` exclusively (ADR-0014).

---

## Background

`BacktestManager` already exposes the full lifecycle:
`create`, `list`, `get`, `stop`, `delete`, `rerun`
(`crates/backtest/src/manager.rs`). The REST API wires it through
`apps/platform/src/main.rs` and `crates/api/src/routes/backtests.rs`. The MCP
layer needs to inject it into `McpContext` and expose three tools:
`create_backtest`, `get_backtest`, `list_backtests`.

`BacktestManager::new` requires a ClickHouse URL and a `PgPool`. Both are
optional at MCP startup; if missing, backtest tools return a clear
`service_unavailable` error rather than panicking.

---

## Tasks

### ☐ F-2.1 Add BacktestManager to McpContext — S

**What:** Extend `McpContext` with an optional `Arc<BacktestManager>`:

```rust
pub backtest_manager: Option<Arc<BacktestManager>>,
```

In `McpContext::new()`:
- Read `CLICKHOUSE_URL` env var.
- If both `CLICKHOUSE_URL` and `DATABASE_URL` are set, construct
  `BacktestManager::new(clickhouse_url, pg_pool.clone())` and wrap in `Arc`.
- If either is missing, log `tracing::info!` and leave as `None`.

`new_without_db()` always leaves `backtest_manager: None`.

**Dependencies:** Add `backtest = { workspace = true }` to
`crates/mcp-server/Cargo.toml`.

**Files:**
- `crates/mcp-server/src/lib.rs`
- `crates/mcp-server/Cargo.toml`

**Acceptance criteria:**
- `cargo build -p mcp-server-lib` succeeds.
- When `CLICKHOUSE_URL` is unset, MCP starts normally and backtest tools
  return `{ "error": "service_unavailable", "reason": "backtest service not configured" }`.
- When both env vars are set, `BacktestManager` is constructed (connection
  test optional — lazy connection is fine).

---

### ☐ F-2.2 `list_backtests` + `get_backtest` read tools — S

**What:** Two read-only tools that surface backtest state.

**`list_backtests`:**
- Params: none (uses `DEV_USER` until Set E Phase 1 lands)
- Calls `BacktestManager::list(user_id)` → newest-first list
- Returns:
  ```json
  {
    "backtests": [
      {
        "id": "<uuid>",
        "name": "ema_cross_v1 · BTC-USDT · 1m",
        "status": "Completed",
        "progress": 100.0,
        "instrument_id": "BTC-USDT",
        "timeframe": "1m",
        "start": "2024-01-01T00:00:00Z",
        "end": "2024-03-31T23:59:59Z",
        "created_at": "2026-06-13T12:00:00Z",
        "finished_at": "2026-06-13T12:01:45Z"
      }
    ]
  }
  ```

**`get_backtest`:**
- Params: `backtest_id` (UUID string)
- Calls `BacktestManager::get(user_id, id)` → full `BacktestSnapshot`
- Returns the snapshot including `result` (equity curve, trade log, metrics)
  and `coverage` (expected_bars, present_bars, missing_ranges).
- Returns `{ "error": "not_found" }` if no match.

**Files:**
- `crates/mcp-server/src/tools/backtests.rs` (new file)
- `crates/mcp-server/src/tools/mod.rs`
- `crates/mcp-server/src/lib.rs`

**Acceptance criteria:**
- Both tools appear in `tool_definitions()`.
- `list_backtests` returns `{ "backtests": [] }` (not an error) when no
  runs exist.
- `get_backtest` with unknown id returns `{ "error": "not_found" }`.

---

### ☐ F-2.3 `create_backtest` write tool — M

**What:** Triggers a new backtest run. An agent can reference either a
persisted strategy (by `store_id`) or a freshly-finalized draft (same
`store_id` returned by `finalize_strategy`), combine it with instrument and
date-range params, and receive a `backtest_id` to poll.

**Tool:** `create_backtest`

Params:
| Param | Type | Required | Notes |
|-------|------|----------|-------|
| `store_id` | UUID string | yes | From `create_strategy` or `finalize_strategy` |
| `instrument_id` | string | yes | e.g. `"BTC-USDT"` |
| `asset_class` | string | yes | e.g. `"crypto_spot_cex"` |
| `timeframe` | string | yes | `"1s"` \| `"1m"` \| `"5m"` \| `"15m"` \| `"1h"` \| `"4h"` \| `"1d"` |
| `start` | RFC3339 string | yes | e.g. `"2024-01-01T00:00:00Z"` |
| `end` | RFC3339 string | yes | |
| `name` | string | no | Display name; defaults to `"<strategy_id> · <instrument> · <timeframe>"` |
| `initial_balance` | decimal string | no | Default `"100000"` |
| `quote_currency` | string | no | Default `"USD"` |
| `auto_collect` | bool | no | Default `true` |

**Implementation:**
1. Look up `store_id` in `ctx.strategy_store` → get `StrategyDefinition`.
2. Parse and validate params (timeframe key, date ordering, instrument_id
   non-empty).
3. Construct `ResolvedSpec` and call `BacktestManager::create(DEV_USER, spec)`.
4. Return `{ "backtest_id": "<uuid>", "status": "Queued" }`.

**Error cases:**
- `store_id` not found → `{ "error": "strategy_not_found" }`
- Invalid timeframe → `{ "error": "invalid_timeframe", "valid_values": [...] }`
- `end` ≤ `start` → `{ "error": "invalid_date_range" }`
- Backtest service unavailable → `{ "error": "service_unavailable" }`

**Files:**
- `crates/mcp-server/src/tools/backtests.rs`
- `crates/mcp-server/src/lib.rs`

**Acceptance criteria:**
- A valid `create_backtest` call returns a UUID and `"status": "Queued"`.
- Subsequent `get_backtest` with that UUID shows status progressing.
- All error cases return the structured error JSON (not a 500/panic).
- `timeframe: "weekly"` → invalid_timeframe error listing the 7 valid keys.
