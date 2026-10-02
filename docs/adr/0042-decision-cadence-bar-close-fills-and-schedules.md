# ADR-0042: Decision Cadence — Bar Close, Own Fills and Schedules; Never Every Tick

**Status:** Accepted
**Date:** 2026-10-01
**Deciders:** platform architect (owner delegated "when does a strategy think")
**Supersedes:** per-trade-tick strategy evaluation in hot-path stage 3
**Traces:** [FEAT-007](../specs/FEAT-007-unified-strategy-engine.md) §7; QA-1, QA-6

## Context

Hot-path stage 3 evaluates every strategy on every `WorldEvent`, which today is every trade tick.
Backtests run on bars. A decision taken between two bar closes on live ticks has no counterpart in
a bar backtest, so per-tick evaluation is a parity break by construction: the backtest cannot
reproduce what paper did, whatever else is shared.

## Decision

A strategy is evaluated on exactly these triggers:

| Trigger | Trading | Selection |
|---|---|---|
| close of a bar of the strategy's decision timeframe | default | default |
| one of its own orders filling or changing state | opt-in `on_fill` | — |
| a schedule (cron, exchange-calendar aware) | opt-in | opt-in |

Never on every tick. **Protection is not a decision**: stops, targets and trailing stops are resting
venue orders (or venue-emulated), so they react at tick speed on paper and live, and on the bar's
high/low path in a backtest. Models are called only on decision ticks.

## Considered options

- **Every tick** — fastest reaction; unreproducible from bars; model calls per tick are unaffordable.
- **Bar close only** — reproducible, but cannot react to an entry filling (to place protection).
- **Bar close + own fills + schedule** — chosen; every trigger has an exact backtest counterpart.

## Consequences

- Good: every decision a live strategy takes can be replayed in a backtest (ADR-0047).
- Good: model and CPU cost scale with bars, not ticks.
- Bad: a strategy cannot react to an intrabar move except through resting orders. Recorded as a
  deliberate trade-off; sub-minute decision timeframes remain available where sub-minute bars exist.
