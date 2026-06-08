# 10 — Open Questions

These are undecided and **gate** the corresponding code. "Design decided" elsewhere in these docs
does not mean "built and tested." Answer the gating items before writing the code they block.

## Gating (answer before the named code)

### Q1 — Real money or paper first? (gates the execution layer)
**Recommendation:** paper/simulated execution first, behind the same execution interface that a
live broker adapter will implement. Flip to live only after the risk gate, kill switch, and
reconciliation are **built and tested**. Stocks imply a regulated broker with real compliance
surface; crypto may be more permissive. This answer shapes the broker adapter and the strictness
of the risk gate.

### Q2 — Which broker(s)/venue(s) for v1? (gates collectors + execution adapters)
Pick one crypto venue and one equity broker to start. Their exact WS/REST shapes determine the
first `normalize()` implementations and the first execution adapter. Build the **second collector
deliberately differently from the first** so the lane/metadata abstraction is *proven*, not
papered over.

### Q3 — Strategy definition format v1.0 freeze (gates all three front doors)
The format in [04-strategy-system.md](./04-strategy-system.md) is a sketch. It must be pinned to
`1.0` before the visual builder, JSON API, or MCP server are built, because all three target it
and changing it later breaks users' strategies. Decide: expression language for conditions, node
types, how `$each` fan-out works, how `risk_overrides` are validated as tighten-only.

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
