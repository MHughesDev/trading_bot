# ADR-0046: The Risk Gate Runs in Every Environment, Including Backtests

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect
**Extends:** ADR-0005 (single risk gate chokepoint) from live and paper to backtest
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §9.3; COMP-002 §3.1; QA-1, QA-5

## Context

COMP-002 drew one chokepoint: every order, from a person or a strategy, passes `RiskGate::check`
before execution. Paper honours it. The backtester does not call it at all, so a backtest can report
profits that came from orders the live gate would have refused — a position above the cap, a burst
over the order-rate limit, trading after the daily-loss trip.

## Decision

Every order from every strategy in every environment passes the same `RiskGate::check`, with that
environment's context: in a backtest, simulated equity, positions, clock and a per-run kill switch.
Rejections are first-class backtest events, counted in results and visible per trade; the daily-loss
trip halts the simulated strategy exactly as it would halt a live one.

## Consequences

- Good: a backtest cannot be better than the system would allow the strategy to be.
- Good: the gate's own behaviour is exercised by every backtest, not only by live traffic.
- Bad: backtests that relied on ungated behaviour report worse numbers. That is the point; the
  difference is reported per run when a translated v1 strategy is re-run.
