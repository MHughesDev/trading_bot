# AI Agent MCP Platform — Full Read/Write Strategy, Backtest & Automation — Set F

**Completion: 9 / 11 shipped, 2 deliberately reversed — CLOSED, SUPERSEDED by
ADR-0022 + AGENT-006 (audited 2026-09-15)**

> **Do not execute this set as written.** The `0 / 11` above was a bookkeeping
> failure, not a build failure: Set F was **built during the ADR-0022 work in
> September 2026 and never ticked off**. `INTG-001` has said
> `Status: Implemented · Version: 2.0 (Set F)` since that work landed, which is
> the clearest single piece of evidence that this MASTER was stale rather than
> the code being absent.
>
> Re-checked against the code on 2026-09-15:
>
> | # | Task | State | Evidence / disposition |
> |---|---|---|---|
> | 1 | **F-0.1** stdio → Streamable HTTP (MCP 2025-03-26) | **Shipped** | `apps/mcp-server/src/main.rs:1` module doc declares Streamable HTTP; `POST /mcp` at `:57`, SSE upgrade at `:117-166`, keep-alive/`notifications/progress` streaming at `:173-236`. Exceeds the task: ADR-0022 §3 added keep-alives the plan never asked for. |
> | 2 | **F-1.1** `StrategyDraft` store in `McpContext` | **Shipped** | `crates/mcp-server/src/lib.rs:188` `draft_store: Arc<Mutex<HashMap<Uuid, StrategyDraft>>>`; `new_strategy_draft` / `discard_draft` in `taxonomy.rs`. ADR-0022 §1 kept the draft builder process-local *by name* — this is the one part of `McpContext` the ADR preserved. |
> | 3 | **F-1.2** 7 step-by-step builder tools | **Shipped, exceeded** | `builder` namespace holds **10** tools (`taxonomy.rs:64-74`): the 7 planned plus `new_strategy_draft`, `discard_draft`, `get_draft_summary`. |
> | 4 | **F-1.3** `finalize_strategy` (assemble → validate → persist) | **Shipped** | `taxonomy.rs:74`; end-to-end test `workflow_a_draft_to_finalize_persists_via_api` (`crates/mcp-server/tests/integration.rs:261`) and the negative case `workflow_b_invalid_draft_catches_errors` (`:303`). |
> | 5 | **F-2.1** Add `BacktestManager` to `McpContext` | **Reversed — do not build** | ADR-0022 §1 names this exact design as the defect it was written to remove: a second `BacktestManager` over the shared `backtest_jobs` table whose hydrate pass marks non-terminal rows failed. `McpContext` (`lib.rs:183-191`) now holds only `ApiClient`, `draft_store`, `trajectory`. |
> | 6 | **F-2.2** `list_backtests` + `get_backtest` | **Shipped, exceeded** | `taxonomy.rs:76-87` — plus `stop_backtest`, `rerun_backtest`, `delete_backtest`, `compare_backtests`, `wait_for_backtest`. Tests at `integration.rs:393, 414, 465`. |
> | 7 | **F-2.3** `create_backtest` write tool | **Reversed — do not build** | Deleted under INV-16: it dispatched compute with no ledger trial row. The refusal is now an enforced invariant, not an absence — `taxonomy.rs:78-80` records why, and two tests prove it stays gone: `dispatching_create_backtest_by_name_is_refused` (`integration.rs:367`) and `no_tool_profile_can_dispatch_an_unregistered_backtest` (`:661`). The legitimate successor is `backtest_strategy`, which registers the trial first. AGENT-006 P5 restates this as a catalogue principle. |
> | 8 | **F-3.1** Automation storage wired into `McpContext` | **Shipped differently** | Storage is **not** in `McpContext`; automations route over HTTP to `/api/automations` (`tools/automations.rs:14, 73, 99`), which is ADR-0022 §1's thin-client rule. The task's *outcome* — the agent can reach automations — holds; its *mechanism* is correctly dead. |
> | 9 | **F-3.2** `list_automations` | **Shipped** | `taxonomy.rs:93`. |
> | 10 | **F-3.3** `create_automation`, `arm_automation`, `disarm_automation` | **Shipped, with the live gate** | `taxonomy.rs:94-99`. `MCP_ALLOW_LIVE_AUTOMATIONS` defaults off exactly as decision 6 locked it (`lib.rs:279-281`, `tools/automations.rs:45`), proven by `live_automations_default_off` (`integration.rs:586`) and `automations_route_through_api_and_live_is_gated` (`:496`). `arm_automation` was additionally classified `Risk::Destructive` — stricter than the plan. |
> | 11 | **F-4.1 / F-4.2** `tool_definitions()`, INTG-001, integration tests | **Shipped; forward ownership moves to AGENT-006** | `INTG-001` header reads `Status: Implemented · Version: 2.0 (Set F)`. 681-line `integration.rs` with 17 tests, plus 7 taxonomy tests. **But** the hand-maintained four-array registration this task institutionalised is precisely what AGENT-006 §0 and §2.3 identify as a defect source and replace with `ToolSpec`. Further work on tool declaration belongs to AGENT-006, not here. |
>
> **Disposition: closed.** Nine tasks shipped (two of them exceeded), two were
> deliberately reversed by decisions that outrank this plan. Nothing in Set F is
> still owed. Roadmap gap **G-5** is resolved: there is nothing to re-derive,
> because the only live question — *how a tool is declared and how the catalogue
> grows* — already has an owner in
> [`AGENT-006`](../../../specs/AGENT-006-tool-catalog.md). Human gate **H-5**'s
> Set F half needs no decision.
>
> **Two things this audit found that are not Set F's to fix, recorded so they are
> not lost:**
>
> 1. `taxonomy.rs:90` records that *the agent's service token does not carry
>    `automations.arm`*, so the automation tools are unreachable from a research
>    session regardless of policy. They exist for the human MCP client only. That
>    is a deliberate and good choice, but it means **F-3.x shipped a surface the
>    internal agent cannot use** — worth knowing before anyone plans work on top
>    of it.
> 2. `INTG-001` §2 still says the server "binds to loopback only until Set E
>    Phase 1 (cookie-session auth) lands", carrying a Set-E dependency that Set
>    E's own audit must confirm. The bind guard is a **Set E** item, not an MCP
>    one.
>
> The sections below are kept for the decision record in §"Locked Decisions",
> which documents why the shipped surface looks the way it does. Read them as
> history.

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
| 0 | [phase-0.md](phase-0.md) | HTTP Transport | 1 | **100%** | Shipped — Streamable HTTP + SSE keep-alives |
| 1 | [phase-1.md](phase-1.md) | Strategy Builder | 3 | **100%** | Shipped — 10 builder tools (7 planned), draft store, finalize |
| 2 | [phase-2.md](phase-2.md) | Backtest Tools | 3 | **33% + 2 reversed** | Read tools shipped; F-2.1 and F-2.3 reversed by ADR-0022 / INV-16 |
| 3 | [phase-3.md](phase-3.md) | Automation Tools | 3 | **100%** | Shipped over HTTP, live mode gated off |
| 4 | [phase-4.md](phase-4.md) | Spec & Tests | 2 | **100%** | INTG-001 v2.0 Implemented; 17 integration + 7 taxonomy tests |

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
| 2026-09-15 | all | audit | **Set closed.** Audited against the code by the plan-hygiene track (ROADMAP §9 G-5). 9 of 11 tasks were found already shipped and never ticked; F-2.1 and F-2.3 are deliberately reversed by ADR-0022 §1 and INV-16 and must not be built. Forward ownership of tool declaration passes to AGENT-006. See the superseded header for per-task evidence. |
