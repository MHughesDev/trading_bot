# Phase 3 — Automation Tools

**Completion: 0% (0 / 3 tasks)**

**Goal:** Let an AI agent create an automation that ties a strategy to an
instrument, then arm or disarm it. Uses the existing automation storage layer
(`crates/storage/src/automation.rs`) and the `pg` pool already in `McpContext`.

---

## Background

Automations are persisted as `AutomationRow` records in Postgres (JSONB `spec`
column). The storage layer has full CRUD:
`insert_automation`, `list_automations`, `set_automation_armed`, `delete_automation`.

The REST API (`crates/api/src/routes/automations.rs`) already exposes these
operations. The MCP layer wires the same storage functions into new tools,
with one extra gate: `create_automation` with `account_mode: "live"` is blocked
unless `MCP_ALLOW_LIVE_AUTOMATIONS=true` (env flag, default `false`). Paper
mode is unrestricted.

The current user identity is `Uuid::nil()` (`DEV_USER`) until Set E Phase 1
lands — same as the REST API.

---

## Tasks

### ☐ F-3.1 Wire automation storage into McpContext — S

**What:** `McpContext` already has `pg: Option<PgPool>`. No new field is
needed — automation functions take `&PgPool` directly. This task just adds
`storage` as a dependency to `crates/mcp-server/Cargo.toml` and verifies
the storage functions compile from the MCP crate.

Add `MCP_ALLOW_LIVE_AUTOMATIONS` env flag:
```rust
pub fn mcp_live_automations_allowed() -> bool {
    std::env::var("MCP_ALLOW_LIVE_AUTOMATIONS")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}
```
Expose this helper from `crates/mcp-server/src/lib.rs`.

**Files:**
- `crates/mcp-server/Cargo.toml` — add `storage = { workspace = true }`
- `crates/mcp-server/src/lib.rs`

**Acceptance criteria:**
- `cargo build -p mcp-server-lib` succeeds.
- `MCP_ALLOW_LIVE_AUTOMATIONS=false` (default) is verifiable in a unit test.

---

### ☐ F-3.2 `list_automations` read tool — S

**What:** Lists all automations for the current user.

**Tool:** `list_automations`
- Params: none
- Calls `storage::automation::list_automations(pg)` (all records, newest first)
- Returns:
  ```json
  {
    "automations": [
      {
        "id": "<uuid>",
        "kind": "single_instrument",
        "account_mode": "paper",
        "armed": false,
        "spec": {
          "asset_class": "crypto_spot_cex",
          "instrument_id": "BTC-USDT",
          "execution_strategy_id": "<strategy-uuid>",
          "time_window": { "start": null, "end": null, "timezone": "UTC" }
        },
        "created_at": "2026-06-13T12:00:00Z"
      }
    ]
  }
  ```
- Returns `{ "automations": [] }` (not an error) when pg is unavailable or
  no automations exist. Log a warning if pg is `None`.

**Files:**
- `crates/mcp-server/src/tools/automations.rs` (new file)
- `crates/mcp-server/src/tools/mod.rs`
- `crates/mcp-server/src/lib.rs`

**Acceptance criteria:**
- `list_automations` appears in `tool_definitions()`.
- Returns empty list (not error) when no automations exist.
- Serialised `spec` is the full `AutomationSpec` JSONB blob from the DB row.

---

### ☐ F-3.3 `create_automation`, `arm_automation`, `disarm_automation` write tools — M

**What:** Three write tools covering the full automation write surface for
`SingleInstrument` automations (Pipeline automations are a Phase 4 / Set E
future-scope item).

**`create_automation`:**

Params:
| Param | Type | Required | Notes |
|-------|------|----------|-------|
| `execution_strategy_id` | UUID string | yes | Must exist in `strategy_store` |
| `instrument_id` | string | yes | e.g. `"BTC-USDT"` |
| `asset_class` | string | yes | e.g. `"crypto_spot_cex"` |
| `account_mode` | string | yes | `"paper"` \| `"live"` |
| `armed` | bool | no | Default `false` |
| `time_window_start` | HH:MM string | no | Trading window open; `null` for 24/7 |
| `time_window_end` | HH:MM string | no | Trading window close |
| `time_window_tz` | IANA tz string | no | Default `"UTC"` |

**Implementation:**
1. Validate `execution_strategy_id` exists in `ctx.strategy_store`.
2. If `account_mode == "live"` and `!mcp_live_automations_allowed()` → return
   `{ "error": "live_automations_disabled", "hint": "Set MCP_ALLOW_LIVE_AUTOMATIONS=true" }`.
3. Build `AutomationSpec::SingleInstrument { ... }`.
4. Build `AutomationRow` with `user_id = DEV_USER`, `kind = "single_instrument"`,
   `spec = serde_json::to_value(spec)`, `armed = params.armed`.
5. Call `storage::automation::insert_automation(pg, &row)`.
6. Return `{ "automation_id": "<uuid>", "armed": false, "kind": "single_instrument" }`.

**`arm_automation`:**
- Params: `automation_id` (UUID)
- Calls `storage::automation::set_automation_armed(pg, id, true)`
- Returns `{ "automation_id": "<uuid>", "armed": true }` or `{ "error": "not_found" }`

**`disarm_automation`:**
- Params: `automation_id` (UUID)
- Calls `storage::automation::set_automation_armed(pg, id, false)`
- Returns `{ "automation_id": "<uuid>", "armed": false }` or `{ "error": "not_found" }`

**Error cases (create_automation):**
- `execution_strategy_id` not in strategy_store → `{ "error": "strategy_not_found" }`
- `account_mode: "live"` with flag off → `{ "error": "live_automations_disabled" }`
- DB unavailable → `{ "error": "service_unavailable" }`

**Files:**
- `crates/mcp-server/src/tools/automations.rs`
- `crates/mcp-server/src/lib.rs`

**Acceptance criteria:**
- `create_automation` with `account_mode: "live"` and default env returns the
  `live_automations_disabled` error (not a panic).
- `create_automation` with `account_mode: "paper"` succeeds and the returned
  `automation_id` appears in a subsequent `list_automations` call.
- `arm_automation` + `list_automations` shows `armed: true`.
- `disarm_automation` + `list_automations` shows `armed: false`.
- Unknown `automation_id` in arm/disarm returns `{ "error": "not_found" }`.
