# 10 — Open Questions

These are undecided and **gate** the corresponding code. "Design decided" elsewhere in these docs
does not mean "built and tested." Answer the gating items before writing the code they block.

## Decided (no longer blocking)

### Q1 — Real money or paper first? → **DECIDED: paper first**
Paper/simulated execution via the existing paper adapter. Flip to live broker APIs only after the
risk gate, kill switch, and reconciliation are proven. Live crypto (Coinbase) and live equities
(Alpaca) use the same execution interface — switching is an adapter swap, not a rewrite.

### Q2 — Which broker(s)/venue(s) for v1? → **DECIDED: Coinbase + Alpaca**
- **Crypto:** Coinbase Advanced Trade (WebSocket market data + REST execution).
- **Equities:** Alpaca (WebSocket market data + REST/WebSocket execution).
- **Data granularity:** 1-minute OHLCV bars for both (see [03-data-engineering.md §11](./03-data-engineering.md)).
  Sub-minute and order-book data are architected for but not populated in MVP.
- Kraken is the next logical crypto venue (after MVP); its collector is built deliberately
  differently from Coinbase to prove the abstraction.

### Q9 — Backtest engine ownership? → **DECIDED: external (market_simulator)**
This repo does not own a fill simulator or replay engine. Backtesting is delegated to
`github.com/MHughesDev/market_simulator` via the `crates/market-simulator-adapter`. The
adapter exports raw archived events as Arrow IPC, submits Run Requests, and translates results.
See [07-storage-and-replay.md](./07-storage-and-replay.md).

### Q10 — Strategy scoping: multi-asset or per-asset? → **DECIDED: per-asset instances**
A strategy definition is asset-class-scoped but not pre-bound to an instrument. A strategy
instance is created when a user clicks an asset and hits "Initialize." One instance per
instrument per user at a time (MVP UX). The runtime is multi-instance capable; the UX enforces
the constraint. `$each` fan-out is removed from the format; replaced by `$bound_at_init`.

## Gating (answer before the named code)

### Q3 — Strategy definition format v1.0 freeze (gates all three front doors)
The format in [04-strategy-system.md](./04-strategy-system.md) is a sketch. It must be pinned to
`1.0` before the visual builder, JSON API, or MCP server are built, because all three target it
and changing it later breaks users' strategies. Decide: expression language for conditions, node
types, `$bound_at_init` semantics, `asset_class` scoping rules, how `risk_overrides` are
validated as tighten-only.

### Q4 — Capital/liability model (gates anything touching real funds)
Run by friends, on a private network, potentially against real accounts. Who is liable when a
friend's strategy loses money? Whose credentials/accounts are used? This is a non-technical
decision that nonetheless constrains the risk posture and the auth/permission model. Resolve
before any real-money switch (couples with Q1).

## Important but not blocking the first crate

### Q5 — Auth model for the trusted group
Local + trusted, but private user data (orders, positions, balances, credentials) must still be
scoped per user on the wire. Decide the minimal auth that enforces per-subscription authorization
without building multi-tenant machinery.

### Q6 — Backtest execution fidelity
How is slippage/queue-position/partial-fill modeled in paper execution? Determines how much to
trust backtests. Can start crude and improve, but document the assumptions so results aren't
overtrusted.

### Q7 — Watermark defaults per source
2s for liquid CEX data is a starting default; equity vs crypto vs (future) on-chain want
different values, stored in instrument metadata. Tune against real feeds.

### Q8 — Retention policy
How long raw events live in object storage vs ClickHouse vs hot cache. Affects cost and how far
back backtests can reach.

## Standing reminder

Every "decided mechanism" (quarantine, revisions, idempotent fills, no-lookahead replay,
reconciliation halts) needs a **test that proves it fires** against adversarial input (malformed
feed, duplicate delivery, late trade, ack timeout, broker desync). Decided ≠ done.
