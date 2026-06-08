# ADR-0008: available_time Ordering and Same Builders for Live and Replay

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Platform team

## Context

Lookahead bias is the single most common reason backtests lie. A strategy that accidentally receives a future bar — even one millisecond in the future — produces performance results that are impossible to replicate live. Backtests built on lookahead bias overstate performance, sometimes dramatically, and lead directly to deploying strategies that fail in production.

The problem is not only about timestamps. The deeper risk is **divergence between live and replay pipelines**: if the bar builder, feature engine, or order-book reconstructor has a separate implementation for backtesting versus live trading, the two can diverge in subtle ways — different rounding, different watermark handling, different event ordering — that make the backtest results misleading even without explicit lookahead.

Two mechanism decisions are required:

1. **Which timestamp is the clock?** The `event_time` (when the source says it happened) is not safe — it does not account for processing delay. A feature computed at `T` is only available at `T + processing_delay`. If the strategy is handed the event at `event_time`, it is handed data it could not have had in real life.

2. **How are builders shared between live and backtest?** If there are two codepaths — one for live consumption of the bus and one for replaying archived events — they will diverge. The only way to guarantee identical behavior is to have one codepath.

## Decision

**`available_time` is the authoritative clock for strategy and backtest access.** Every event carries an `available_time` field representing when a strategy or backtest is allowed to use that event. `available_time` includes all processing delays: a 1-minute bar covering `10:30:00–10:30:59.999` is built and published at `10:31:02` (after the configured watermark); `10:31:02` is its `available_time`. A feature computed from that bar at `10:31:02.050` has `10:31:02.050` as its `available_time`. A strategy can never receive an event before that event's `available_time`.

**The replay engine sorts strictly by `available_time` and advances a single simulated clock.** Dequeuing is gated: an event is only handed to the strategy when the simulated clock reaches its `available_time`. It is structurally impossible to hand a strategy an event from its own future because the dequeue loop will not advance past the current clock position.

**Bar builders, feature engines, and order-book reconstructors are pure functions with no I/O**, implemented in `crates/builders` and `crates/features`. They consume an ordered stream of events and emit derived events. They have no database connections, no clocks, no side effects.

- **Live path:** these pure functions consume the NATS JetStream bus.
- **Replay path:** the market_simulator (external) feeds them the recorded raw normalized events from the Parquet archive, in `available_time` order, through the same function code.

There are never two implementations of a builder — one for live and one for replay. There is one implementation, and both paths use it.

## Rationale

`available_time` as the authoritative clock eliminates lookahead bias structurally rather than by policy. It is impossible to hand a strategy something it could not have known at the time because the event's `available_time` records the exact moment it became knowable, including all processing pipeline delays. This is not an approximation — it is computed forward from `event_time + watermark + observed processing delay` and stamped on the event at creation.

Sorting replay strictly by `available_time` means the simulated strategy sees events in exactly the order a live strategy would have seen them, including the interleaving of delayed features with the raw market events that preceded them.

Pure function builders eliminate the divergence risk between live and replay. A function with no I/O, no clock access, and no mutable global state produces identical output for identical input regardless of whether that input comes from a live bus subscription or a file-backed replay. The correctness guarantee is mechanically enforced by the type system (`crates/builders` and `crates/features` have no `tokio::time`, no database imports, no NATS imports in their dependency trees).

## Consequences

**Positive:**
- Lookahead bias is structurally impossible: the replay dequeue gate enforces `available_time` ordering mechanically.
- Live and replay use identical builder and feature code; divergence between live performance and backtest performance attributable to different pipelines is eliminated.
- `available_time` is a persistent field on every stored event, so the guarantee holds for events replayed years after they were recorded.
- Feature processing delays are automatically encoded into `available_time` at compute time; strategies do not need to manually account for feature staleness.
- Pure function builders are trivially testable: call with input events, assert output events, no mocks needed.

**Negative:**
- `available_time` computation must be correct for every event type. A bug in the watermark calculation or the feature pipeline's `available_time` stamping will silently produce incorrect replay ordering. Each mechanism needs an adversarial test.
- The watermark delay (2s default for liquid CEX data, configurable per source) means strategies see bars 2 seconds later than the bar's close time. Latency-sensitive strategies must accept this delay or explicitly opt into unconfirmed pre-watermark events.
- Pure functions cannot perform I/O, which means any builder that requires state across event windows (e.g., an EMA over 200 bars) must carry its state as an explicit parameter. This makes builder state management explicit and slightly more verbose.

**Neutral:**
- The `available_time` of a feature event is typically slightly later than the `available_time` of the raw event that triggered its computation. This is correct: the feature was not available until it was computed.
- The watermark is configurable per source in instrument metadata. Tighter watermarks (e.g., 0.5s for a highly liquid venue) reduce latency at the cost of potentially missing more late data. The default of 2s is a conservative starting point.

## Alternatives Considered

### Option A: Use `event_time` as the Strategy Clock
Hand strategies events timestamped with `event_time` (when the source says the event happened). Simpler: no `available_time` computation required.

Not chosen because: `event_time` does not account for processing delay. A feature that required 100ms to compute from a bar is handed to the strategy at the bar's `event_time`, appearing as if it was available before it was computed. This is definitional lookahead bias. In backtesting, this produces overstated performance that cannot be replicated live.

### Option B: Separate Builder Implementations for Live and Backtest
Maintain one bar builder for live consumption and a separate, simpler bar builder for backtest data export.

Not chosen because: two implementations will diverge. Differences in rounding, edge-case handling, or watermark application between the two versions will cause the backtest to produce different bars than the live system, making backtest results unreliable as predictors of live performance. The divergence risk is not hypothetical — it is the documented failure mode of most backtest systems.

### Option C: Apply `available_time` Only to Features, Not Raw Events
Use `event_time` for raw market events and `available_time` only for computed features.

Not chosen because: raw market events can also arrive late (trades with delayed confirmation, feed reconnect replays). Using `event_time` for raw events and `available_time` for features creates an inconsistent ordering model where a strategy might receive a feature at `T+100ms` computed from a raw event it has not yet received. The unified `available_time` field on all events — raw and derived — eliminates this inconsistency.

## References

