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
| Q-2 | Which broker(s)/venue(s) for v1? | Resolved | Coinbase, Alpaca, Kraken, others | Coinbase = live execution; Alpaca = paper execution + equity data; Kraken = crypto market data; market_simulator = backtest execution. Venue routing via `crates/venue-router`. | ADR-0006, ADR-0011 |
| Q-3 | Strategy definition format v1.0 freeze | Open | Expression language, node types, `$bound_at_init` semantics, `asset_class` scoping, `risk_overrides` validation | — | Gates all three front doors (visual builder, JSON API, MCP server). Must be resolved in Phase 0. |
| Q-4 | Capital/liability model | Open | Shared account vs. per-user accounts; liability allocation when a friend's strategy loses money | — | Gates any real-money switch. Couples with Q-1. Post-Phase-2 scope. |
| Q-5 | Auth model for trusted group | Open | JWT-based per-user scoping vs. minimal shared-secret vs. other | — | Local + trusted, but private user data (orders, positions, balances, credentials) must be scoped per user on the wire. |
| Q-6 | Backtest execution fidelity | Open | Crude slippage model vs. queue-position model vs. historical spread replay | — | Determines how much to trust backtests. Can start crude but assumptions must be documented so results aren't overtrusted. |
| Q-7 | Watermark defaults per source | Open | 2s for liquid CEX (starting default); per-instrument tuning from metadata | — | 2s default in place; equity vs. crypto vs. on-chain want different values. Tune against real feeds. |
| Q-8 | Retention policy | Open | How long raw events live in object storage vs. ClickHouse vs. hot cache | — | Affects cost and how far back backtests can reach. |
| Q-9 | Backtest engine ownership | Resolved | Build in-repo vs. delegate to external library | External: `market_simulator` (`github.com/MHughesDev/market_simulator`). This repo owns only `crates/market-simulator-adapter`. | ADR-0009 area |
| Q-10 | Strategy scoping: multi-asset or per-asset? | Resolved | Multi-asset strategy instances vs. per-asset instances | Per-asset instances with `$bound_at_init`. A strategy instance is created when a user clicks "Initialize" on an asset. One instance per instrument per user (MVP UX). `$each` fan-out removed. | [spec/10-open-questions.md](../refactor_reference_docs/spec/10-open-questions.md) |

> Number questions sequentially: `Q-1`, `Q-2`, … Append new ones at the end; never renumber.

---

## Entry Detail

### Q-3: Strategy definition format v1.0 freeze

**Status:** Open
**Gates:** Phase 5 (visual builder, JSON API, MCP server) — all three front doors target this format. Changing it after any front door is built breaks users' strategies.

**Why it matters:** The strategy definition format in `spec/04-strategy-system.md` is a sketch. It must be pinned to `1.0` before the visual builder, JSON API, or MCP server are built. All three front doors produce the same canonical strategy definition document — if the format is unfrozen, none of them can be built safely.

**Decisions required:**
- Expression language for conditions
- Node types (complete enumeration)
- `$bound_at_init` semantics (what can and cannot be declared, how the runtime resolves it at initialize time)
- `asset_class` scoping rules (which fields are asset-class-specific vs. universal)
- How `risk_overrides` are validated as tighten-only (not looser than the global risk gate)

**What would decide it:** A Phase 0 spec task that pins the format to 1.0 with at least one working round-trip test (definition → runtime → signal). The Phase 5 tasks are blocked until that spec is `Implemented`.

**Resolution:** [To be filled in at Phase 0 completion — the frozen format version and the spec/ADR that carries it.]

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
