# DATA-003: Timestamps and Identity

**Status:** Draft
**Version:** 0.1
**ADR(s):** ADR-0008
**Success Conditions:** SC-3, SC-4

## 1. Purpose

Defines the four-timestamp model carried by every `EventEnvelope<T>` and the deterministic event identity (dedup key) scheme used across all stream types. `available_time` is the most consequential field in the system: it is the clock the replay engine advances, it is what prevents lookahead bias, and its identical computation in both live and replay is what makes "same strategy, same result" a structural guarantee rather than a hope.

## 2. Scope & Non-Goals

**In scope:**
- Semantics of all four timestamps: `event_time`, `observed_time`, `ingested_time`, `available_time`.
- How `available_time` is computed (watermark + processing delay).
- Why `available_time` is the replay clock and how it prevents lookahead bias.
- Dedup key derivation per stream type (sequenced streams, trades, on-chain events).
- The append-only/revision model as it relates to identity (revision events have new `available_time`s).

**Not in scope (deliberate):**
- The watermark policy values (2s default, configurable per source) — specified in COMP-001.
- The full `EventEnvelope<T>` struct — specified in DATA-001.
- Bar builder logic and the mechanism that stamps `available_time` on derived events — specified in COMP-001.
- Clock synchronization between collector hosts — infrastructure concern, not a data model concern.

## 3. Design

### 3.1 The Four Timestamps

Every `EventEnvelope<T>` carries up to four timestamps:

| Field | Type | Meaning |
|-------|------|---------|
| `event_time` | `Option<DateTime<Utc>>` | When the source says the event happened. Optional because some sources do not provide it. |
| `observed_time` | `DateTime<Utc>` | When our collector first received the raw bytes from the venue. Always present; set by the collector process before publishing. |
| `ingested_time` | `DateTime<Utc>` | When the normalized event entered the internal bus. Set by the ingestion layer after `normalize()` succeeds. |
| `available_time` | `DateTime<Utc>` | **When a strategy or backtest is allowed to use this event.** The replay clock field. Always present. |

### 3.2 available_time — The Load-Bearing Field

`available_time` is computed to include all processing delay between the event happening and a strategy being permitted to act on it. Examples:

- A raw trade at `10:30:00.050` with `observed_time = 10:30:00.070` and a 30ms normalization pass has `available_time ≥ 10:30:00.100`.
- A 1-minute bar covering `10:30:00–10:30:59.999` is built after a 2-second watermark and stamped `available_time = 10:31:02.000` (or later if the bar builder ran behind).
- A technical indicator computed from that bar (e.g. EMA) is stamped with `available_time` equal to the time the feature engine finished computing it — identically in live and in replay.

The invariant:

> A strategy receives event E at simulation time T only when `T >= E.available_time`. The replay engine advances its simulated clock by dequeuing events in strict `available_time` order. It is structurally impossible for a strategy to receive an event from its own future.

This is the mechanism that prevents lookahead bias — the single most common reason backtests lie.

### 3.3 available_time Computation Rules

| Event type | available_time formula |
|------------|------------------------|
| Raw trade / quote (live, not derived) | `max(ingested_time, observed_time + normalization_latency_estimate)` |
| OHLCV bar (original, `revision: 0`) | `interval_end + watermark_duration` for the source |
| OHLCV bar (revision, `revision > 0`) | `time_revision_was_computed` — later than the original's `available_time` |
| Derived feature (indicator, model output) | `time_feature_engine_finished_computing` |
| Order-book snapshot / delta | `observed_time + normalization_latency_estimate` |

The watermark duration is configurable per source in instrument metadata (default: 2 seconds for liquid centralized-exchange data). See COMP-001 for the full watermark policy.

### 3.4 Lookahead Bias Prevention — Structural Guarantee

The replay engine in COMP-004 uses a single simulated clock that advances by dequeuing events sorted strictly by `available_time`. Because:

1. Feature engines stamp `available_time` to include their own processing delay — identically live and in replay.
2. The replay loop will not dequeue an event until the simulated clock reaches its `available_time`.
3. Strategies read only from `WorldContext`, which is populated exclusively from dispatched events.

...lookahead is structurally impossible. A strategy cannot call `world.feature(instrument, "ema_7")` and receive a value whose `available_time` is in the strategy's future. The same pipeline that enforces this live enforces it in replay because it is the same code.

### 3.5 Event Identity and Dedup Keys

A timestamp alone is never a valid primary key for market events. The deterministic dedup key is derived from the source, never generated randomly at ingest time. This is critical: collector reconnects, JetStream redeliveries, and overlapping snapshot+delta windows will all produce duplicate events.

| Stream type | Dedup key fields |
|-------------|-----------------|
| Sequenced streams (order book deltas, bars from sequenced feeds) | `lane + instrument_id + venue_id + sequence + source` |
| Trades | `venue_id + exchange_trade_id` |
| On-chain events (future) | `chain + tx_hash + log_index` |

These keys serve double duty:
- **Storage dedup:** ClickHouse `ReplacingMergeTree` is ordered on these keys for eventual-consistency dedup.
- **Live-path dedup:** A Redis seen-set keyed on these values is checked at the storage writer for the live path, preventing duplicate inserts in the short window before ClickHouse merge.

For high-frequency streams, `sequence` numbers in the envelope are mandatory for ordering and gap detection (see COMP-001 §7 for gap handling).

### 3.6 Revision Events and Identity

When late data arrives after the watermark, a revision event is emitted rather than mutating the original event. The revision:

- Shares the same logical bar key (same `interval_start`, `interval_end`, `instrument_id`).
- Has `revision: u32 > 0` on `BarPayload`.
- Has a new `event_id` (it is a distinct event).
- Has a new `available_time` (the time the revision was computed — always later than the original's `available_time`).

The original is immutable. The revision is a new immutable fact that supersedes it. A backtest can replay both the original and the revision at their true `available_time`s, reproducing exactly what a live strategy saw — including whether it acted on the original before the revision arrived.

## 4. Interfaces

**Timestamps are fields of `EventEnvelope<T>`** — see DATA-001 §3.1 for the full struct.

**The replay clock (`available_time`) is the sort key** used by the replay engine when reading from the Parquet archive. See COMP-004 §3 for the Arrow IPC export schema.

**`world.now()` in strategy code** returns the current `available_time` of the most recently dispatched event — not the wall clock. Strategies must never call wall-clock functions directly; this is enforced by convention (and eventually a lint rule).

**Dedup key computation** is performed in the collector's `normalize()` function and set on `EventEnvelope.sequence` (for sequenced streams) or embedded in `TradePayload.exchange_trade_id`. The storage writer derives the storage key from these fields.

## 5. Dependencies

- DATA-001 — `EventEnvelope<T>` struct that carries all four timestamps and `sequence`.
- DATA-002 — instrument metadata provides `trading_hours` context for freshness watchdog decisions.
- COMP-001 — watermark policy values and bar builder `available_time` stamping.
- COMP-004 — replay engine that sorts the event stream by `available_time`.
- FEAT-001 — strategy runtime that reads `world.now()` (the dispatched `available_time`) rather than the wall clock.

## 6. Acceptance Criteria

- [ ] AC-1: In a replay run, no event is dispatched to a strategy before the simulated clock reaches its `available_time` — verified by checking that every `WorldEvent` delivered to `on_event` has `available_time <= world.now()` — Verified by: [—]
- [ ] AC-2: A 1-minute bar with `interval_end = T` has `available_time >= T + watermark_duration` for the configured source watermark — Verified by: [—]
- [ ] AC-3: A revision bar (`revision: 1`) for an interval has `available_time` strictly greater than the original bar's (`revision: 0`) `available_time` for the same interval — Verified by: [—]
- [ ] AC-4: Two `TradePayload` envelopes with identical `venue_id + exchange_trade_id` are identified as duplicates at the storage writer and only one is persisted — Verified by: [—]
- [ ] AC-5: Two `EventEnvelope` values with identical sequenced-stream dedup keys (`lane + instrument_id + venue_id + sequence + source`) are identified as duplicates at the storage writer and only one is persisted — Verified by: [—]
- [ ] AC-6: `world.now()` inside a strategy's `on_event` callback returns the `available_time` of the event being dispatched, not the OS wall clock — Verified by: [—]

## 7. Open Questions

None at this revision. Watermark values are an operational parameter specified in COMP-001 and configurable per source in instrument metadata.
