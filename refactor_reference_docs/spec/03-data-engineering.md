# 03 — Data Engineering

Correctness lives here. A perfect event fabric still loses money if a float touches a price or a
late trade poisons a bar. Every failure mode below has a **decided mechanism** — but "decided"
means *design decided*, not *built and tested*. Each mechanism needs a test that proves it fires.

## 1. Schema-on-write + quarantine

Validate at ingest. A collector's `normalize()` returns `Result<Vec<EventEnvelope>, NormalizeError>`.
Anything that fails validation goes to a **`quarantine` lane** — not dropped, not coerced — with
the raw bytes and the error attached, so it can be replayed after the normalizer is fixed.

When a venue changes a field without warning:
1. Normalizer fails → messages land in quarantine.
2. The lane's freshness watchdog fires within seconds (the lane went quiet).
3. A human patches the normalizer, bumps the payload `schema_version`, replays quarantine.

Blast radius: one venue's lane is stale for minutes; **zero corrupt rows reach storage**.

## 2. Decimals (see also 02)

`Decimal` from wire to warehouse. `Price`/`Size` newtypes with no `From<f64>`. Per-instrument
precision from instrument metadata. The compiler enforces the rule; it won't build if a float
touches a price.

## 3. Dedup + idempotency

You **will** receive every message more than once (collector reconnect+replay, JetStream
redelivery, overlapping snapshot+delta).

- **Deterministic dedup key** derived from the source (see identity in
  [02-data-model.md](./02-data-model.md)), never a random UUID at ingest.
- ClickHouse tables use `ReplacingMergeTree` ordered on that key (eventually-consistent dedup),
  plus a short-window seen-set in Redis at the writer for the live path.
- **Money-mutating consumers are idempotent by construction.** Applying fill `F` to a position is
  keyed on `F`'s id; processed-fill-ids are recorded; replaying `F` is a no-op. The risk gate is
  idempotent too, or a redelivery double-submits.

## 4. Watermarks + late / out-of-order data (the policy)

Data arrives late and out of order. The fatal choice is **not deciding**. The decided policy:

- **Bars carry a watermark** — 2 seconds for liquid centralized-exchange data, **configurable
  per source** in instrument metadata. A 1-minute bar for `10:30:00–10:30:59.999` is built and
  published at `10:31:02`; that is its `available_time`.
- Trades arriving **before** the watermark go into the bar.
- Trades arriving **after** the watermark do **not** silently mutate the published bar. Instead
  a **revision event** (`market.bars.1m.revised`, same bar key, `revision: 1`, new
  `available_time`) is emitted. The original is immutable; the revision is a new immutable fact
  that supersedes it.

Strategies choose a policy:
- Latency-sensitive: act on the first bar, accept it may be marginally wrong, ignore revisions.
- Correctness-sensitive: wait for the watermark or consume revisions.

Either way the data layer **never rewrites history** — it only appends, and supersession is
explicit and timestamped. This is exactly what makes backtests reproducible.

## 5. No lookahead — the mechanism

The replay engine sorts strictly by `available_time` and advances a single simulated clock. A
strategy can only be handed an event **when the clock reaches that event's `available_time`**. It
is structurally impossible to hand a strategy something from its own future, because the loop
will not dequeue it yet. Feature engines stamp `available_time` to include their own processing
delay, **identically live and in replay**.

## 6. Same builders, live and replay

The deepest divergence risk is "live and backtest run different pipelines." We **don't run
different pipelines.** Bar builders, feature engines, and order-book reconstructors are pure
functions over event streams, living in their own crates.

- **Live:** they consume the bus.
- **Replay:** the replay engine feeds them the **recorded raw normalized events** from the
  archive, in `available_time` order, through the *same builder code*.

The raw normalized event archive (Parquet/object store) is the **ground truth** — written
**before** any derivation, append-only, immutable. History is never recomputed a different way
than live; it is re-fed through identical code. That's why "same strategy, same result" is a
guarantee, not a hope.

## 7. Reconciliation (scheduled, not heroic)

- **Position** vs broker: on every fill, on a 30-second sweep, and on every reconnect. On
  disagreement → risk gate halts new orders on that instrument and alarms.
- **Bar volume** vs raw-trade volume: nightly job.
- **Sequence gaps**: detected live in every collector → triggers snapshot re-request and emits
  `gap.detected` so downstream knows that window is suspect.
- **Freshness watchdogs** per lane: if a lane is quiet beyond its expected interval *during
  trading hours for that instrument*, alarm. Reads instrument metadata so a normal stock close at
  4pm does not false-alarm.

Correctness observability sits alongside throughput observability in the same metrics stack.

## 8. Source trust tiers

Not all sources are equally trustworthy. Trust is a first-class field on every event and
instrument.

| Tier | Examples (v1 + future) | Quality posture |
|------|------------------------|-----------------|
| `regulated` | stocks, ETFs, bonds | mostly trusted; respect halts/auctions |
| `centralized_exchange` | Binance, Coinbase | gap detection, sanity bounds |
| `onchain_confirmed` | confirmed swaps (future) | confirmation-gated |
| `onchain_tentative` | sub-confirmation swaps (future) | reorg handling, `is_tentative` |
| `social_derived` | sentiment (future) | bot/spam filtering before influencing features |

Quality gates scale with the tier. Meme-coin/DEX data (future) is the dirtiest source you'll ever
ingest — reorgs, failed txns masquerading as trades, wash trading, honeypots, 18-decimal prices —
and must be far more defensive than a regulated exchange. Strategies can declare a **minimum
trust tier** they will act on.

## 9. Partitioning for reads

Optimize physical layout for the **dominant** query; accept the others are slower.

- **Backtest** (one instrument, long range): ClickHouse `ORDER BY (instrument, available_time)`,
  partitioned by month → contiguous reads, no universe scan.
- **Live latest-state**: Redis, keyed `latest:{lane}:{instrument}` — never a warehouse scan.
- **Research matrices** (cross-asset): Parquet partitioned `lane/instrument/date` with DataFusion
  predicate pushdown.
- **Small-files problem**: the storage writer batches (10k events or 100ms, whichever first); a
  nightly compaction job rolls small files into big ones.

## 10. The four properties everything rests on

Append-only, deterministic, idempotent, validated-at-write. These compose. A system built on them
does not get *more* fragile as the tenth asset class is added — it gets *more* proven, because
every new source flows through the same hardened path.

## 11. Data granularity model

The system is designed around three data granularity levels that can coexist on the bus as
independent lanes. They are different subscriptions, not mutually exclusive:

| Level | Lane | Frequency | Notes |
|-------|------|-----------|-------|
| OHLCV bars | `market.bars.1m`, `market.bars.1s` | 1/minute, 1/second | Primary for MVP |
| Order-book deltas | `market.orderbook.l2` | Many per second | Post-MVP for v1 venues |
| Individual trades | `market.trades` | Per-trade | Post-MVP for v1 venues |

**MVP constraint:** Coinbase and Alpaca's APIs are not well-suited to reliable sub-minute
streaming data. The v1 MVP operates on **1-minute OHLCV bars** as the primary data plane for
live strategy execution and backtest export.

**Architecture is granularity-agnostic:** The bus lanes, the bar builder, the strategy runtime,
and the storage layer do not assume 1-minute as the only granularity. Second-level bars and
order-book level data are valid payload types in the schema from day one; they simply will not be
populated by the v1 collectors. When a venue's API supports reliable higher-frequency data (or
when Kraken/other venues are added), those lanes activate with no schema changes.

**Strategies declare what they need.** A strategy definition declares its minimum bar timeframe
in `inputs`. A strategy written for 1m bars runs unmodified if 1s bars become available and the
demand is re-declared. A strategy that requires order-book data cannot be initialized on an asset
that only has bar data — the validator rejects it.
