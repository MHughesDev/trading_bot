# Open Questions

A living register of questions the system has not yet resolved. This is not a scratchpad —
it is the durable record of every consequential fork the project has faced, how it was decided,
and what evidence decided it.

## When to add an entry

Add an open question **whenever you compare important parts of the system or weigh trade-offs
between variations** — choosing a data store, an API to integrate, a pattern, a boundary between
components. If a decision is worth making deliberately, the question behind it is worth recording here.

## How entries move

1. **Open** — the question is raised. Record the options on the table and what would decide between them.
2. **Researching** — a research brief is underway (link it).
3. **Resolved** — the question is answered. Record the answer, the evidence that settled it, and the
   ADR or spec that now carries the decision.

Never delete a resolved question. The point of this file is that six months from now, anyone can see
not just *what* was decided but *why*, and which alternatives were already ruled out.

---

## Register

| ID | Question | Status | Options Weighed | Resolution | Evidence / Links |
|----|----------|--------|-----------------|------------|------------------|
| Q-1 | Real money or paper first? | Resolved | Live broker vs. paper/simulated execution | Alpaca paper account for all paper trading (all assets/domains). Live crypto (Coinbase) and live equities (Alpaca) use the same execution interface — switching is an adapter swap, not a rewrite. | ADR-0006 |
| Q-2 | Which broker(s)/venue(s) for v1? | Resolved | Coinbase, Alpaca, Kraken, others | Coinbase = live execution; Alpaca = paper execution + equity data; Kraken = crypto market data. Venue routing via `crates/venue-router`. | ADR-0006, ADR-0011 |
| Q-3 | Strategy definition format v1.0 freeze | Resolved | Expression language, node types, `$bound_at_init` semantics, `asset_class` scoping, `risk_overrides` validation | Format frozen at v1.0 in Phase 0; all three front doors (visual builder, JSON API, MCP server) built against it in Phase 5. `ValidatedDefinition` sealed type enforces validation before use. | ADR-0007, DATA-004, `crates/strategy-validator` |
| Q-4 | Capital/liability model | Open (deferred) | Shared account vs. per-user accounts; liability allocation when a friend's strategy loses money | Deferred until live-money switch is planned. No operator agreement reached; private network scope means the question is acknowledged but non-blocking. | — |
| Q-5 | Auth model for trusted group | Resolved (MVP) | JWT-based per-user scoping vs. minimal shared-secret vs. other | Bearer token auth implemented in `crates/api` for MVP. Full JWT per-user scoping is the planned upgrade path; deferred until real-money onboarding. | `crates/api/src/auth.rs` |
| Q-6 | Backtest execution fidelity | Closed (out of scope) | Crude slippage model vs. queue-position model vs. historical spread replay | Backtesting removed from repo scope entirely (2026-06-10). Revisit only if backtesting returns as a project much later. | — |
| Q-7 | Watermark defaults per source | Resolved | 2s for liquid CEX (starting default); per-instrument tuning from metadata | `watermark_secs` is a field on `Instrument` (default 2). Phase 6 confirmed: equity instruments use 2s same as crypto. Tune per-instrument via the instruments table. | `crates/domain/src/instrument.rs`, `crates/storage/src/postgres/instruments.rs` |
| Q-8 | Retention policy | Open (deferred) | How long raw events live in object storage vs. ClickHouse vs. hot cache | No retention limits set. Raw events in Parquet/NATS JetStream grow unbounded until manually pruned. Deferred to operational scaling concern. | — |
| Q-9 | Backtest engine ownership | Closed (out of scope) | Build in-repo vs. delegate to external library | Backtesting removed from repo scope entirely (2026-06-10). `crates/market-simulator-adapter`, backtest REST endpoints, and MCP backtest tools deleted. | — |
| Q-10 | Strategy scoping: multi-asset or per-asset? | Resolved | Multi-asset strategy instances vs. per-asset instances | Per-asset instances with `$bound_at_init`. A strategy instance is created when a user clicks "Initialize" on an asset. One instance per instrument per user (MVP UX). `$each` fan-out removed. | ADR-0007, FEAT-001 |

> Number questions sequentially: `Q-1`, `Q-2`, … Append new ones at the end; never renumber.

---

## Entry Detail

### Q-3: Strategy definition format v1.0 freeze

**Status:** Resolved — Phase 5 complete (2026-06-08)

**Resolution:** Format frozen at `definition_version: "1.0"` in Phase 0. Implemented in `crates/domain/src/strategy_def/`. All three front doors (REST API, visual builder, MCP server) produce and validate the same canonical v1.0 JSON via `crates/strategy-validator`. `ValidatedDefinition` is a sealed type — cannot be constructed outside the validator crate, so no front door can bypass validation. Evidence: `strategy-validator` test suite passes; DATA-004 spec is `Implemented`.

---

### Q-4: Capital/liability model

**Status:** Open
**Gates:** Any real-money switch. This is post-Phase-2 scope and not in the current plan's execution path.

**Why it matters:** The system is run by friends on a private network, potentially against real accounts. The non-technical question of who is liable when a strategy loses money directly constrains the risk posture and the auth/permission model. Whose broker credentials are used? Are positions tracked per-user or shared? This decision cannot be deferred indefinitely once live trading is on the table.

**Decisions required:**
- Whose accounts/credentials are used for live execution
- Liability allocation (operator vs. strategy author vs. shared)
- Whether per-user capital limits are enforced at the risk gate level
- How the permission model reflects the liability answer

**What would decide it:** A non-technical agreement among operators, documented before the real-money switch is enabled. The outcome constrains the `crates/risk` limits model and the auth scoping in Q-5.

**Resolution:** [To be filled in before any real-money switch is enabled.]
