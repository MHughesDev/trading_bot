# ADR-0039: One Deterministic Strategy Engine — Environments Are Adapters

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect, on the owner's direction that strategies "omit themselves exactly
the same in backtesting and paper trading and live trading"
**Extends:** ADR-0038 (one *live* evaluation path → one evaluation path, full stop)
**Supersedes:** the evaluation loop in `backtest/src/sim.rs`, the preview loop in
`backtest/src/preview.rs`, `strategy-runtime/src/interpreter.rs` as an evaluator, and FEAT-005 §4.1
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §3, §9; COMP-002 §3.7; QA-1, QA-2, QA-7

## Context

A strategy is evaluated by three different programs. The backtester runs its own bar loop over a
stateless interpreter; the agent's preview runs a third copy of that loop; paper runs
`StrategyInstance`, which has history registers and model slots the backtester lacks. They have
already drifted: paper can remember the previous bar and the backtester cannot; paper has model
slots (never filled) and the backtester has none; each sizes and exits in its own code; and on
paper the indicators are never computed at all. A good backtest therefore proves nothing about
what the same strategy does on paper.

ADR-0038 solved the same shape once, for the two *live* doors. The backtest side was left out.

## Decision

There is **one strategy engine**, in a new crate `strategy-engine`. It is:

- **pure** — no I/O, no clock reads, no async, no global state;
- **synchronous and deterministic** — the same plan, state and events produce the same decisions,
  bit for bit; outputs never depend on hash-map iteration order;
- **environment-blind** — it receives events (bar closes, fills, order updates, timers) and a
  portfolio view, returns model requests or decisions, and never learns which mode it is in.

Everything that legitimately differs between backtest, paper and live sits behind four ports
supplied by an **Environment**: `Clock`, `MarketData`, `ModelProvider`, and the venue (reached
through the risk gate and execution engine). The engine computes its own indicators from raw data
through the `features` crate in every environment; nothing outside it supplies indicator values.

## Considered options

1. **Keep two evaluators and test them against each other.** Rejected: the comparison is the
   thing that keeps failing; two implementations of one semantics is the defect, not the
   mitigation.
2. **Drive `StrategyInstance` from the backtester (FEAT-005 §4.1).** Closer, but `StrategyInstance`
   is bound to the v1 definition format, reads an externally supplied feature slot array, and its
   model slots are pushed in from outside — the exact seams where paper broke.
3. **A new pure engine; environments as adapters.** Chosen.

## Consequences

- Good: parity holds by construction, and is *proved* nightly by journal replay (ADR-0047).
- Good: the paper "indicators never fire" defect disappears structurally — there is no external
  feature feed to forget.
- Good: preview becomes a short backtest run; the third evaluator is deleted, not reconciled.
- Bad: model calls, which are asynchronous, need a request/resume protocol (ADR-0043).
- Bad: `strategy-runtime` lives on beside the new crate until migration completes (strangler fig);
  a source-scan guard prevents new callers of the retired evaluators in the meantime.
