# COMP-001: Data Quality and Ingestion

**Status:** Draft
**Version:** 0.1
**ADR(s):** ADR-0003, ADR-0009, ADR-0011
**Success Conditions:** SC-3, SC-6

## 1. Purpose

Defines the correctness mechanisms that run between a collector receiving raw bytes from a venue and a normalized `EventEnvelope<T>` being published to the internal bus. This component owns schema validation and quarantine routing, dedup/idempotency at ingest, watermark policy and late-data handling, revision event emission, trust tier assignment, and collector reconnect behavior. Correctness lives here: a perfect event fabric still loses money if a float touches a price or a late trade silently poisons a bar.

## 2. Scope & Non-Goals

**In scope:**
- `normalize()` contract and quarantine routing for schema failures.
- Freshness watchdog per lane (staleness detection with instrument metadata awareness).
- Watermark policy: default values, configurability per source, and the bar publication timing rule.
- Revision events for late data (`market.bars.1m.revised`).
- Dedup/idempotency: deterministic dedup keys, ClickHouse `ReplacingMergeTree`, Redis seen-set.
- Trust tier table and trust-based quality gates.
- Collector reconnect behavior and sequence-gap detection.
- The four foundational properties: append-only, deterministic, idempotent, validated-at-write.
- Data granularity model (bars primary for MVP; order-book and trades as post-MVP lanes).

**Not in scope (deliberate):**
- Transport protocol for the internal bus (NATS JetStream) — infrastructure concern.
- Storage write batching and ClickHouse partitioning details — specified in COMP-004.
- The bar builder's internal state machine — implementation detail of the bar-builder crate.
- Feature engine computation — a downstream consumer of normalized events.

## 3. Design

### 3.1 Schema-on-Write and the Quarantine Lane

Every collector's `normalize()` function is the ingest gate:

```rust
fn normalize(raw: &[u8]) -> Result<Vec<EventEnvelope<DomainPayload>>, NormalizeError>;
```

- **Success:** one or more well-typed `EventEnvelope<T>` values are published to the appropriate lane.
- **Failure:** the raw bytes plus the `NormalizeError` are published to the `quarantine` lane — never dropped, never coerced, never silently discarded.

The quarantine record carries:
- The raw bytes as received from the venue.
- The `NormalizeError` (structured, not just a string).
- `source`, `venue_id`, `observed_time`.

When a venue changes a field without warning:
1. `normalize()` fails → messages land in `quarantine`.
2. The lane's freshness watchdog fires within seconds (that lane went quiet).
3. A human patches the normalizer, bumps `schema_version`, replays quarantine through the fixed normalizer.

Blast radius: one venue's lane is stale for minutes; zero corrupt rows reach storage.

### 3.2 Decimal Enforcement

`Price` and `Size` newtypes have no `From<f64>` impl. The compiler refuses to build if a float touches a price. Per-instrument precision (from `Instrument.base_precision` / `quote_precision` in DATA-002) is applied in `normalize()` — quantize to the instrument's real precision, never truncate real money away.

### 3.3 Dedup and Idempotency at Ingest

Collectors will deliver every message more than once (reconnect+replay, JetStream redelivery, overlapping snapshot+delta). The dedup strategy is layered:

- **Deterministic dedup key:** derived from the source (see DATA-003 §3.5), never a random UUID at ingest. This key is set on the envelope by the collector's `normalize()`.
- **Live path (short window):** a Redis seen-set keyed on the dedup key, checked at the storage writer. Prevents duplicate inserts before ClickHouse merge catches up.
- **Storage (eventual):** ClickHouse tables use `ReplacingMergeTree` ordered on the dedup key — eventual-consistency dedup for the full history.
- **Money-mutating consumers:** idempotent by construction. Applying fill `F` to a position is keyed on `F`'s id; processed-fill-ids are recorded; replaying `F` is a no-op. The risk gate is idempotent too, or a redelivery double-submits (see COMP-002).

### 3.4 Watermark Policy

Late and out-of-order data are a certainty. The decided policy:

| Source type | Default watermark | Configurable? |
|-------------|-------------------|---------------|
| Liquid CEX (Coinbase, Alpaca) | 2 seconds | Yes, per source in instrument metadata |
| Illiquid or future sources | TBD — set conservatively | Yes |

**Bar publication rule:** A 1-minute bar covering `10:30:00–10:30:59.999` is built and published no earlier than `10:31:02` (interval end + 2s watermark). Its `available_time` is that publication time. Trades arriving before the watermark elapses go into the bar. Trades arriving after do not.

**No silent mutation:** Trades arriving after the watermark do not rewrite the published bar. Instead a revision event is emitted:
- Lane: `market.bars.1m.revised`
- Same bar key (`interval_start`, `interval_end`, `instrument_id`)
- `BarPayload.revision: u32 > 0`
- New `available_time` = time the revision was computed

The original bar is immutable. The revision is a new immutable fact. Both are stored append-only, each at its true `available_time`, so a backtest can reproduce exactly what a live strategy saw — including whether it saw the revision before acting.

Strategies choose their policy:
- **Latency-sensitive:** act on the first bar, accept marginal inaccuracy, ignore revisions.
- **Correctness-sensitive:** wait for the watermark or subscribe to revision events.

### 3.5 No-Lookahead Mechanism

The bar builder and all feature engines stamp `available_time` to include their own processing delay — identically in live and in replay. The replay engine (COMP-004) sorts strictly by `available_time` and advances a single simulated clock. It is structurally impossible to hand a strategy something from its own future because the loop will not dequeue it yet. See DATA-003 §3.2–3.4 for the full invariant.

### 3.6 Same Builders, Live and Replay

The deepest divergence risk is "live and backtest run different pipelines." This system does not:

- Bar builders, feature engines, and order-book reconstructors are **pure functions over event streams**, living in dedicated crates.
- **Live:** they consume events from the bus.
- **Replay:** the replay engine feeds them the recorded raw normalized events from the Parquet archive, in `available_time` order, through the *same builder code*.

The raw normalized event archive is the ground truth — written before any derivation, append-only, immutable. History is never recomputed a different way than live.

### 3.7 Freshness Watchdog

One watchdog per active lane. Reads `trading_hours` and `halt_behavior` from instrument metadata (DATA-002) to distinguish expected silence from broken silence:

| Condition | Action |
|-----------|--------|
| Lane quiet during trading hours (equity in session) | Alarm; halt affected strategies |
| Lane quiet outside trading hours (equity market closed) | No alarm — expected |
| Lane quiet for crypto instrument | Always alarm; halt affected strategies |
| Halt state active on haltable instrument | No alarm; continue monitoring |

### 3.8 Sequence Gap Detection

Every collector tracks the `sequence` field on incoming envelopes. On a detected gap:
1. Emit a `gap.detected` event on the instrument's meta lane (signals downstream the window is suspect).
2. Request a snapshot re-send from the venue if the protocol supports it.
3. The freshness watchdog raises an alarm if the gap is not resolved within the configured tolerance.

### 3.9 Collector Reconnect Behavior

Collectors are satellite processes — they crash and reconnect on their own rhythm and must not take the core down. On reconnect:
1. Re-establish the venue connection.
2. Request a snapshot (for order-book streams) to resync state.
3. Resume publishing normalized events; the dedup layer handles any overlap with previously published events.
4. Emit a `collector.reconnected` meta event so downstream can flag the gap window.

### 3.10 Source Trust Tiers

Trust is a first-class field on every event and instrument. Quality gates scale with the tier:

| Tier | Examples (v1 + future) | Quality posture |
|------|------------------------|-----------------|
| `regulated` | stocks, ETFs, bonds | Mostly trusted; respect halts/auctions |
| `centralized_exchange` | Binance, Coinbase | Gap detection, sanity bounds |
| `onchain_confirmed` | Confirmed swaps (future) | Confirmation-gated |
| `onchain_tentative` | Sub-confirmation swaps (future) | Reorg handling, `is_tentative` flag |
| `social_derived` | Sentiment signals (future) | Bot/spam filtering before influencing features |

Strategies declare a `min_trust_tier`; the strategy validator and risk gate both enforce it. Meme-coin and DEX data (future) is the dirtiest source and must be far more defensive than a regulated exchange.

### 3.11 Data Granularity Model

Three granularity levels coexist on the bus as independent lanes:

| Level | Lane | v1 status |
|-------|------|-----------|
| OHLCV bars | `market.bars.1m`, `market.bars.1s` | **Primary for MVP** — 1m only |
| Order-book deltas | `market.orderbook.l2` | Post-MVP for v1 venues |
| Individual trades | `market.trades` | Post-MVP for v1 venues |

MVP constraint: Coinbase and Alpaca APIs are not well-suited to reliable sub-minute streaming. The v1 MVP operates on 1-minute OHLCV bars. The architecture is granularity-agnostic: second-level bars and order-book data are valid payload types from day one; they simply will not be populated by the v1 collectors.

### 3.12 The Four Foundational Properties

All correctness mechanisms compose around these four properties:

1. **Append-only** — history is never mutated; revisions are new events.
2. **Deterministic** — same raw input always produces the same normalized output and `available_time`.
3. **Idempotent** — every ingest path and money-mutating consumer is safe to replay.
4. **Validated-at-write** — no unvalidated data reaches the bus or storage.

These properties do not weaken as more asset classes are added — each new source flows through the same hardened path.

## 4. Interfaces

**Collector `normalize()` signature:**
```rust
fn normalize(raw: &[u8]) -> Result<Vec<EventEnvelope<DomainPayload>>, NormalizeError>;
```

**Lanes written to by this component:**
- `market.trades` — `TradePayload`
- `market.quotes` — `QuotePayload`
- `market.orderbook.l2` — `OrderBookPayload`
- `market.bars.1m`, `market.bars.1s` — `BarPayload` (`revision: 0`)
- `market.bars.1m.revised` — `BarPayload` (`revision > 0`)
- `quarantine` — raw bytes + `NormalizeError`
- `gap.detected` — meta event (instrument, source, sequence range)
- `collector.reconnected` — meta event (source, venue_id, timestamp)

**Freshness watchdog inputs:**
- `Instrument.trading_hours` (DATA-002)
- `Instrument.halt_behavior` (DATA-002)
- Lane quiet-period threshold (configurable per source)

## 5. Dependencies

- DATA-001 — `EventEnvelope<T>` struct, payload types, dedup key fields.
- DATA-002 — `Instrument` metadata (trading hours, halt policy, trust tier, precision).
- DATA-003 — `available_time` stamping rules and watermark policy.
- COMP-004 — Parquet archive that receives the raw normalized events as ground truth.
- `rust_decimal` crate — decimal enforcement.
- Redis / Valkey — seen-set for live-path dedup.
- NATS JetStream — the bus that carries all lanes.

## 6. Acceptance Criteria

- [ ] AC-1: A collector that receives bytes that fail `normalize()` publishes the raw bytes and structured error to the `quarantine` lane and does not publish to any other lane — Verified by: [—]
- [ ] AC-2: A bar with `interval_end = T` and a 2s watermark has `available_time >= T + 2s` — Verified by: [—]
- [ ] AC-3: A trade arriving after the watermark for its bar's interval causes a revision event to be emitted on `market.bars.1m.revised` and does not mutate the already-published `BarPayload` — Verified by: [—]
- [ ] AC-4: Replaying the same raw bytes through the same `normalize()` function twice produces identical `EventEnvelope` outputs (determinism) — Verified by: [—]
- [ ] AC-5: The freshness watchdog for an equity instrument does not alarm when the instrument's `TradingSchedule` shows the market is closed — Verified by: [—]
- [ ] AC-6: On gap detection, a `gap.detected` meta event is published before the collector attempts a snapshot re-request — Verified by: [—]
- [ ] AC-7: A collector reconnect that replays events already seen does not produce duplicate rows in ClickHouse after `ReplacingMergeTree` merge (or in Redis seen-set in the live window) — Verified by: [—]

## 7. Open Questions

Q-8 (from `open-questions.md`): Retention policy for the raw Parquet archive — how long to keep raw events before tiering to cheaper storage? (Referenced in COMP-004.)
