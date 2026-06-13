# Phase 4 — Spec Compliance & Integration Tests

**Completion: 0% (0 / 2 tasks)**

**Goal:** Document the complete tool surface in the canonical MCP spec, update
`tool_definitions()` for all new tools, and add integration tests that validate
the full end-to-end agent workflow.

---

## Tasks

### ☐ F-4.1 Update `tool_definitions()` + INTG-001 spec — S

**What:** Every tool added in Phases 0–3 must appear in `tool_definitions()`
with a complete `inputSchema` (JSON Schema). The canonical MCP spec file
(`docs/specs/INTG-001-mcp-server.md`) must be updated to describe the new
tool surface and the HTTP transport.

**`tool_definitions()` additions** (`crates/mcp-server/src/lib.rs`):

Builder tools (Phase 1):
- `new_strategy_draft` — no required params
- `set_strategy_meta` — required: `draft_id`, `strategy_id`, `asset_class`; optional: `min_trust_tier`
- `add_strategy_input` — required: `draft_id`, `lane`, `instrument`; optional: `features` (array of strings)
- `add_condition_node` — required: `draft_id`, `node_id`, `expr`
- `add_signal_node` — required: `draft_id`, `node_id`, `when`, `emit`
- `add_strategy_action` — required: `draft_id`, `on_signal`, `side`, `size_mode`, `size`
- `set_risk_overrides` — required: `draft_id`; optional: `max_position`, `max_order_rate_per_minute`, `max_order_rate_per_second`
- `get_draft_summary` — required: `draft_id`
- `discard_draft` — required: `draft_id`
- `finalize_strategy` — required: `draft_id`

Backtest tools (Phase 2):
- `list_backtests` — no params
- `get_backtest` — required: `backtest_id`
- `create_backtest` — required: `store_id`, `instrument_id`, `asset_class`, `timeframe`, `start`, `end`; optional: `name`, `initial_balance`, `quote_currency`, `auto_collect`

Automation tools (Phase 3):
- `list_automations` — no params
- `create_automation` — required: `execution_strategy_id`, `instrument_id`, `asset_class`, `account_mode`; optional: `armed`, `time_window_start`, `time_window_end`, `time_window_tz`
- `arm_automation` — required: `automation_id`
- `disarm_automation` — required: `automation_id`

**INTG-001 spec updates** (`docs/specs/INTG-001-mcp-server.md`):
- Replace stdio transport description with Streamable HTTP (POST `/mcp`,
  optional SSE upgrade, `GET /health`).
- Add `McpContext` field inventory (pg, strategy_store, instance_manager,
  draft_store, backtest_manager).
- Add full tool table with params, return shapes, and error codes for all
  17 tools (7 original + 10 new).
- Add the `MCP_ALLOW_LIVE_AUTOMATIONS` flag description.
- Add the loopback-only bind constraint and the condition under which it
  can be relaxed (Set E Phase 1 auth).

**Files:**
- `crates/mcp-server/src/lib.rs`
- `docs/specs/INTG-001-mcp-server.md`

**Acceptance criteria:**
- `tool_definitions()` serialises to valid JSON Schema for all 17 tools.
- INTG-001 spec no longer references stdio.
- All required fields are marked `required` in inputSchema; optional fields
  are not.

---

### ☐ F-4.2 Integration tests for the full agent workflow — M

**What:** Tests that validate the three end-to-end agent workflows the MCP
server is designed to enable:

**Workflow A — Draft → Finalize → Backtest:**
1. `new_strategy_draft` → `draft_id`
2. `set_strategy_meta` (ema_cross_v1, crypto_spot_cex)
3. `add_strategy_input` (market.bars.1m)
4. `add_strategy_input` (features.technical, features: ["ema_7","ema_21"])
5. `add_condition_node` (n1, "feature('ema_7') > feature('ema_21')")
6. `add_signal_node` (n2, when: n1, emit: "long")
7. `add_strategy_action` (on_signal: "long", buy, fixed, "0.01")
8. `finalize_strategy` → `store_id`
9. `create_backtest` (store_id, BTC-USDT, 1m, 2024-01-01…2024-03-31) → `backtest_id`
10. `get_backtest` → status Queued/Simulating/Completed

**Workflow B — Invalid draft catches errors:**
1. `new_strategy_draft`
2. `add_condition_node` (n1, bad expr ">>")
3. `finalize_strategy` → `valid: false`, `errors` non-empty, draft intact
4. Fix expr → `finalize_strategy` → `valid: true`

**Workflow C — Assign automation:**
1. `finalize_strategy` with a valid draft → `store_id`
2. `create_automation` (store_id, BTC-USDT, paper) → `automation_id`
3. `list_automations` → includes new automation with `armed: false`
4. `arm_automation` → `armed: true`
5. `list_automations` → `armed: true`
6. `disarm_automation` → `armed: false`

**Test structure:** Unit tests in `crates/mcp-server/tests/integration.rs` using
`McpContext::new_without_db()` (no real DB/ClickHouse required). Backtest tools
skip `create_backtest` assertions in DB-less tests (return service_unavailable
and assert the error shape, not success).

**Files:**
- `crates/mcp-server/tests/integration.rs` (new file)

**Acceptance criteria:**
- `cargo test -p mcp-server-lib` passes all three workflow tests.
- Test B verifies the draft is still accessible after a validation failure.
- All error shapes in the tests match the exact JSON keys specified in the
  phase-1, -2, -3 task descriptions.
- No `unwrap()` or `expect()` in test code — use `assert!(result.is_ok())` etc.
