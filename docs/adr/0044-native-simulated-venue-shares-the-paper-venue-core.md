# ADR-0044: A Native Simulated Venue Shares the Paper Venue's Core; market_simulator Becomes a Reference Venue

**Status:** Accepted — **confirmed by the owner 2026-10-02** ("yes, move off market_simulator")
**Date:** 2026-10-01
**Deciders:** platform architect, on the owner's direction that backtest and paper "should work
similarly but not exactly the same"
**Supersedes:** ADR-0006 and ADR-0014 **in the role of default backtest venue only**. Their principle
— one execution interface, strategy and risk code never branch on mode — is kept and is what this
ADR implements.
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §9.2; FEAT-005 §3, §9; QA-1, QA-3

## Context

The backtest venue is `market_simulator`, reached through a facade whose order surface is
`SimOrderCommand::Market { side, quantity }` and nothing else, which runs one instrument per
simulation, and which reports no fills, positions or account state back to the host — so the
host keeps its own guess of the position. FEAT-005 §9 specified widening it (limit, stop,
stop-limit, trailing, bracket, cancel, fills back, multi-instrument); none of that was done, and it
lives in another repository.

Meanwhile `PaperTradingEngine` already rests limit and stop-limit orders, tracks positions and
per-asset-class accounts, applies funding and settlement, and fills from marks.

## Decision

Extract a **venue core** from `PaperTradingEngine` — working orders, positions, cash, OCO, fees from
`FeeBook`, per-asset-class accounts — and build two venues on it:

- **SimulatedVenue** (backtest): driven by historical bars on a simulated clock. Market orders fill
  at the next bar's open plus modelled half-spread and slippage; limit and stop orders trigger on
  the following bars' high/low path; when a stop and a target both fall inside one bar, the stop
  wins. `CurrentClose` is available only as a declared optimistic policy.
- **PaperVenue** (paper): the same core driven by live marks and trades.

`market_simulator` stays reachable behind the same `Venue` port as a **reference venue**, used to
cross-check fills on the instruments it supports. It is not the default.

## Considered options

- **Widen the market_simulator facade** (FEAT-005 §9). Keeps a purpose-built engine; but every order
  type and the multi-instrument portfolio backtest that selection strategies need become changes in
  another repository, and backtest and paper would still share no execution code.
- **Native venue on the paper core.** Chosen: full order types and multi-instrument from day one,
  one accounting implementation, and "similar, not identical" made literal — same machinery,
  different clock and price source.

## Consequences

- Good: every intent in ADR-0041 can be backtested.
- Good: portfolio backtests (selection-bound deployments) run on one clock and one account.
- Good: a fill-model change is one place, reviewed against the oracle test.
- Bad: the platform now owns its backtest fill model. Mitigated by the reference venue, the oracle
  test, and Set T's forward reconciliation of paper fills against backtest expectations — the
  measurement that tells us whether the simulator is telling the truth.
- **This reverses a decision the owner made on 2026-06-08.** It was flagged before any of it was built, and the owner confirmed it on 2026-10-02.
