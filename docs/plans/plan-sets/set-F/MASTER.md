# AI Agent MCP Platform — Full Read/Write Strategy, Backtest & Automation — Set F

**Completion: 0% (0 / 11 primary tasks)**

## Overview

Set F extends the existing MCP server (`crates/mcp-server` + `apps/mcp-server`)
from a 7-tool stdio utility into a fully capable AI agent platform. The server
gains:

1. **HTTP transport** — replaces stdio with Streamable HTTP (MCP spec
   2025-03-26, Axum) so any MCP-compliant client (Claude Desktop, Cursor, custom
   agents) can connect over the network.
2. **Strategy builder tools** — step-by-step draft construction. An agent
   creates a draft, adds inputs/nodes/actions incrementally, then calls
   `finalize_strategy` which validates and persists. The builder holds server-side
   draft state keyed by a UUID the agent carries.
3. **Backtest tools** — trigger a run against any persisted or freshly-built
   strategy, poll for status, and read full results (equity curve, trade log,
   coverage report).
4. **Automation tools** — assign a strategy to an instrument, arm or disarm the
   automation.

All new tools go through the same validator, risk gate, and data isolation
as the REST API (`apps/platform`). The MCP server remains a single binary; no
new crate is added.

---

## Guiding Constraints

- **ADR-0007 / ADR-0010 (frozen strategy format).** Builder tools produce
  valid v1.0 `StrategyDefinition` JSON. No new node or action kinds are
  introduced here; use existing `Condition`, `Signal`, `PlaceOrder`.
- **ADR-0008 (live = replay).** `finalize_strategy` runs through
  `strategy-validator` — the same pipeline the REST `create_strategy` uses.
  Byte-identical round-trip is non-negotiable.
- **ADR-0014 (backtest via SDK).** `create_backtest` calls `BacktestManager`
  exclusively; no direct ClickHouse queries from the MCP layer.
- **Auth safety gate.** The HTTP server must bind to loopback (`127.0.0.1`)
  until Set E Phase 1 (cookie-session auth) lands. The guard matches the one
  at `apps/platform/src/main.rs`. Remove it last, only after auth verification
  and TLS are in place.
- **Fail-honest.** `finalize_strategy` returns `ValidationError` items with
  JSON-pointer paths so the agent can fix them and retry. Builder tools never
  silently drop input.
- **Key safety (automations).** `create_automation` with `account_mode: "live"`
  is gated by a config flag (`MCP_ALLOW_LIVE_AUTOMATIONS=true`) that defaults
  off. Paper mode is unrestricted.

---

## Phase Summary

| Phase | File | Label | Tasks | Completion | Goal |
|-------|------|-------|-------|------------|------|
| 0 | [phase-0.md](phase-0.md) | HTTP Transport | 1 | 0% | Replace stdio with Streamable HTTP (MCP 2025-03-26) via Axum |
| 1 | [phase-1.md](phase-1.md) | Strategy Builder | 3 | 0% | Draft store + 7 builder tools + finalize |
| 2 | [phase-2.md](phase-2.md) | Backtest Tools | 3 | 0% | BacktestManager in context + trigger/poll/read |
| 3 | [phase-3.md](phase-3.md) | Automation Tools | 3 | 0% | Assign, arm, disarm automations |
| 4 | [phase-4.md](phase-4.md) | Spec & Tests | 2 | 0% | Update INTG-001; integration tests for full agent workflow |

---

## Item → Phase Map

| # | Item | Phase · Task |
|---|------|--------------|
| 1 | stdio → HTTP (Streamable HTTP, MCP 2025-03-26) | 0.1 |
| 2 | StrategyDraft store in McpContext | 1.1 |
| 3 | 7 builder tools (meta, input, nodes, action, risk) | 1.2 |
| 4 | `finalize_strategy` (assemble → validate → persist) | 1.3 |
| 5 | BacktestManager added to McpContext | 2.1 |
| 6 | `list_backtests` + `get_backtest` read tools | 2.2 |
| 7 | `create_backtest` write tool (strategy_ref or draft_id) | 2.3 |
| 8 | Automation storage wired into McpContext | 3.1 |
| 9 | `list_automations` read tool | 3.2 |
| 10 | `create_automation` + `arm_automation` + `disarm_automation` | 3.3 |
| 11 | Update `tool_definitions()`, INTG-001 spec, integration tests | 4.1–4.2 |

---

## Locked Decisions (2026-06-13)

| # | Decision | Locked Choice |
|---|----------|---------------|
| 1 | MCP server topology | **Extend existing** — no new crate, no separate binary |
| 2 | Transport | **Streamable HTTP, MCP spec 2025-03-26** via Axum. POST `/mcp`, optional SSE upgrade for streaming. stdio binary removed. |
| 3 | Strategy creation mode | **Step-by-step builder tools** — server-side draft keyed by UUID. Agent accumulates nodes/inputs/actions, then calls `finalize_strategy`. |
| 4 | Backtest operations | **Trigger + poll + read** — `create_backtest`, `get_backtest`, `list_backtests`. |
| 5 | Automation writes | **Assign + arm/disarm** — `create_automation`, `arm_automation`, `disarm_automation`. |
| 6 | Live automation gate | **Off by default** — `MCP_ALLOW_LIVE_AUTOMATIONS` env flag; paper mode unrestricted. |
| 7 | Backtest user identity | **`DEV_USER` placeholder** (Uuid::nil()) until Set E Phase 1 auth lands. Same as the REST API's current state. |

---

## Recommended Sequencing

Phase 0 → 1 → 2 → 3 → 4. Phase 0 (HTTP transport) is the prerequisite for all
others. Phases 1–3 are independent once Phase 0 lands and can be parallelised.
Phase 4 documents and tests the full surface.

---

## Progress Log

Update this table and the per-phase completion headers as tasks land.

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-13 | — | plan | Set F created; all 7 decisions locked. 11 tasks across 5 phases. |
