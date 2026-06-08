# DATA-001: Event Envelope and Payloads

**Status:** Implemented
**Version:** 1.0
**ADR(s):** ADR-0002, ADR-0009
**Success Conditions:** SC-1, SC-7

## 1. Purpose

Defines the universal `EventEnvelope<T>` wrapper that surrounds every event on the internal bus, the versioned v1 payload types it carries, and the money-safe numeric newtypes (`Price`, `Size`) that all payloads use. This is the foundational data contract from which everything — storage, strategies, replay — is derived.

## 2. Scope & Non-Goals

**In scope:**
- `EventEnvelope<T>` struct definition and field semantics.
- v1 payload types: `TradePayload`, `QuotePayload`, `OrderBookPayload`, `BarPayload`.
- `Price` and `Size` newtypes and their compiler-enforced no-float rule.
- Dedup key derivation rules per stream type.
- `revision` field semantics on `BarPayload` (original vs. superseding revision event).
- `schema_version` field as the compiled-in schema registry.

**Not in scope (deliberate):**
- Transport encoding (JSON vs. Protobuf vs. Arrow) — not irreversible; decided separately.
- Which bus (NATS JetStream) carries these envelopes — see COMP-001 and SYS-001.
- Timestamp semantics — covered in DATA-003.
- Instrument metadata fields referenced from the envelope — covered in DATA-002.
- Payload types for asset classes beyond v1 (options, futures, DEX) — additive extension.

## 3. Design

### 3.1 EventEnvelope

A common outer wrapper surrounds every event. Payloads are typed and versioned. The `domain` crate owns all types; the compiled types are the schema registry — no separate registry service is needed.

```rust
pub struct EventEnvelope<T> {
    pub event_id:       Uuid,                  // unique per event instance
    pub event_type:     String,                // e.g. "market.bar.closed"
    pub schema_version: String,                // payload schema version e.g. "1.0"
    pub lane:           String,                // routing lane / topic
    pub instrument_id:  Option<String>,        // FK into instrument metadata
    pub venue_id:       Option<String>,
    pub source:         String,                // collector / origin id

    pub trust_tier:     TrustTier,             // see COMP-001 §3 for tier table

    // Timestamps — see DATA-003 for full semantics
    pub event_time:     Option<DateTime<Utc>>, // when the source says it happened
    pub observed_time:  DateTime<Utc>,         // when we first saw it
    pub ingested_time:  DateTime<Utc>,         // when it entered the bus
    pub available_time: DateTime<Utc>,         // when a strategy/backtest MAY use it

    pub sequence:       Option<i64>,           // source sequence for ordering/dedup
    pub correlation_id: Option<Uuid>,
    pub causation_id:   Option<Uuid>,
    pub payload:        T,
}
```

### 3.2 v1 Payload Types

#### TradePayload

```rust
pub struct TradePayload {
    pub price:              Price,
    pub size:               Size,
    pub side:               AggressorSide,
    pub exchange_trade_id:  String,
}
```

#### QuotePayload (L1)

```rust
pub struct QuotePayload {
    pub bid:      Price,
    pub bid_size: Size,
    pub ask:      Price,
    pub ask_size: Size,
}
```

#### OrderBookPayload

Designed to express both crypto-style sequenced deltas and stock-style book updates. Differences are in metadata, not in code paths.

```rust
pub struct OrderBookPayload {
    pub kind:         BookUpdateKind,  // Snapshot | Delta
    pub bids:         Vec<Level>,      // (price, size); size 0 = remove level
    pub asks:         Vec<Level>,
    pub sequence:     i64,             // for gap detection
    pub is_tentative: bool,            // true for sub-confirmation on-chain (future); false for v1
}
```

#### BarPayload

```rust
pub struct BarPayload {
    pub timeframe:      Timeframe,
    pub interval_start: DateTime<Utc>,
    pub interval_end:   DateTime<Utc>,
    pub open:           Price,
    pub high:           Price,
    pub low:            Price,
    pub close:          Price,
    pub volume:         Size,
    pub trade_count:    u64,
    pub revision:       u32,  // 0 = original; >0 = supersedes a prior bar
}
```

### 3.3 Money Types: Decimal Everywhere

Prices and sizes are never floats (`0.1 + 0.2 != 0.3`). Newtypes have no `From<f64>` impl so a float cannot compile its way into a price:

```rust
pub struct Price(pub Decimal);
pub struct Size(pub Decimal);
```

- Storage: Postgres `NUMERIC`, ClickHouse `Decimal128(scale)`.
- Per-instrument precision comes from the instrument metadata table (`base_precision`, `quote_precision` in DATA-002).
- Normalization quantizes to the instrument's real precision — never truncate real money away.

### 3.4 Dedup Key Derivation

Event identity is deterministic, derived from the source — never a random UUID at ingest time:

| Stream type | Dedup key |
|-------------|-----------|
| Sequenced streams (order book, bars) | `lane + instrument_id + venue_id + sequence + source` |
| Trades | `venue_id + exchange_trade_id` |
| On-chain events (future) | `chain + tx_hash + log_index` |

These keys are also the primary key for `ReplacingMergeTree` ordering in ClickHouse and the Redis seen-set key on the live path.

### 3.5 Revision Semantics

History is never mutated. Late data does not edit a published bar — it emits a revision event (`revision > 0`) that supersedes the original. The original stays immutable. Both the original and the revision are stored at their true `available_time`s, so a backtest can reproduce exactly what a strategy saw live, including whether it saw a revision before acting.

## 4. Interfaces

**Produced by:** All collector processes via their `normalize()` function, which returns `Result<Vec<EventEnvelope<T>>, NormalizeError>`.

**Consumed by:**
- Strategy runtime (via the canonical bus, never the UI feed) — see FEAT-001.
- Storage writers (ClickHouse, Parquet archive) — see COMP-004.
- UI streaming gateway (lossy consumer) — see COMP-003.
- Data quality layer (quarantine routing) — see COMP-001.

**Lane naming convention:**
- `market.trades` — `TradePayload`
- `market.quotes` — `QuotePayload`
- `market.orderbook.l2` — `OrderBookPayload`
- `market.bars.1m`, `market.bars.1s` — `BarPayload`
- `market.bars.1m.revised` — `BarPayload` with `revision > 0`
- `quarantine` — raw bytes + `NormalizeError` (see COMP-001)

**Trust tier values (on `TrustTier`):**
`regulated` | `centralized_exchange` | `onchain_confirmed` | `onchain_tentative` | `social_derived`

## 5. Dependencies

- `rust_decimal` crate — for `Decimal` backing `Price` and `Size`.
- `uuid` crate — for `event_id`, `correlation_id`, `causation_id`.
- `chrono` crate — for `DateTime<Utc>` timestamp fields.
- DATA-002 (`instrument_id` FK, precision values for quantization).
- DATA-003 (timestamp semantics: `available_time` definition).
- COMP-001 (quarantine lane — where failed `normalize()` results land).
- SYS-001 (lane routing, bus topology).

## 6. Acceptance Criteria

- [x] AC-1: A `Price` or `Size` value constructed from an `f64` literal does not compile — Verified by: Compile-time: `Price(Decimal)` / `Size(Decimal)` — no `From<f64>` impl; `cargo build` fails if you write `Price(1.5f64)`
- [x] AC-2: An `EventEnvelope<BarPayload>` with `revision: 0` and a subsequent `EventEnvelope<BarPayload>` with `revision: 1` sharing the same bar key can both be stored and retrieved without either being mutated — Verified by: `builders::bar::tests::late_data_revision_supersedes_original`
- [x] AC-3: The dedup key for a `TradePayload` envelope is derived solely from `venue_id + exchange_trade_id`; two envelopes with identical keys are treated as duplicates at ingest — Verified by: `collectors::kraken::tests::dedup_key_derivation` (or note: dedup key is `trade_key(venue_id, exchange_trade_id)` in `crates/domain/src/lib.rs`)
- [x] AC-4: A `TradePayload` envelope with `schema_version: "1.0"` fails deserialization if a required field is absent, and the raw bytes are routed to the quarantine lane — Verified by: `quarantine_replay` integration test
- [x] AC-5: `BarPayload.trade_count` is a `u64` and `BarPayload.revision` is a `u32`; both are present in all serialized bar events — Verified by: `domain` compile-time; struct definition in `crates/domain/src/payloads/bar.rs`
- [x] AC-6: `OrderBookPayload` with `kind: Delta` and a level where `size == 0` causes that level to be removed from the reconstructed order book state — Verified by: `builders::order_book::tests::delta_remove_level`

## 7. Open Questions

None at this revision. Timestamp semantics are delegated to DATA-003; trust tier behavior to COMP-001.
