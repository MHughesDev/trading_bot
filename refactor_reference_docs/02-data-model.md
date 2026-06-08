# 02 — Data Model (the irreversible core)

This is the part most expensive to get wrong. The event schema, the timestamp semantics, and the
instrument metadata model are effectively **tattoos**: everything stored depends on them forever.
Spend real time here; stay deliberately loose on transport (which bus, JSON vs binary) because
that is a change of clothes.

## Event envelope (universal)

A common outer envelope wraps every event; payloads are typed and versioned.

```rust
pub struct EventEnvelope<T> {
    pub event_id:       Uuid,                  // unique per event instance
    pub event_type:     String,                // e.g. "market.bar.closed"
    pub schema_version: String,                // payload schema version
    pub lane:           String,                // routing lane / topic
    pub instrument_id:  Option<String>,        // FK into instrument metadata
    pub venue_id:       Option<String>,
    pub source:         String,                // collector / origin id
    pub trust_tier:     TrustTier,             // see 03-data-engineering.md

    // Timestamps — see "Timestamp model" below
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

## v1 payload types

Versioned structs in the `domain` crate **are** the schema registry — compiled, validated, no
separate registry service to operate.

```rust
pub struct TradePayload {
    pub price: Price,           // newtype over Decimal — never f64
    pub size:  Size,            // newtype over Decimal
    pub side:  AggressorSide,
    pub exchange_trade_id: String,
}

pub struct QuotePayload {       // L1
    pub bid: Price,
    pub bid_size: Size,
    pub ask: Price,
    pub ask_size: Size,
}

/// Order-book payload designed to express BOTH crypto-style sequenced deltas
/// AND stock-style book updates. Differences are in metadata, not in code paths.
pub struct OrderBookPayload {
    pub kind: BookUpdateKind,   // Snapshot | Delta
    pub bids: Vec<Level>,       // (price, size); size 0 = remove level
    pub asks: Vec<Level>,
    pub sequence: i64,          // for gap detection
    pub is_tentative: bool,     // true for sub-confirmation on-chain (future); false for v1
}

pub struct BarPayload {
    pub timeframe:      Timeframe,
    pub interval_start: DateTime<Utc>,
    pub interval_end:   DateTime<Utc>,
    pub open:  Price,
    pub high:  Price,
    pub low:   Price,
    pub close: Price,
    pub volume: Size,
    pub trade_count: u64,
    pub revision: u32,          // 0 = original; >0 = supersedes a prior bar (late data)
}
```

### Money types: Decimal everywhere, enforced by the compiler

Prices and sizes are **never** floats (`0.1 + 0.2 != 0.3`). Use newtypes with **no
`From<f64>`** so a float cannot compile its way into a price:

```rust
pub struct Price(pub Decimal);
pub struct Size(pub Decimal);
```

Storage: Postgres `NUMERIC`, ClickHouse `Decimal128(scale)`. Per-instrument precision comes from
the instrument metadata table (BTC size to 8 dp, some token to 18, a stock to 2). Normalization
quantizes to the instrument's real precision — never truncate real money away.

## Instrument metadata (the unsung hero of scalability)

This table is what makes "stocks + crypto today, options tomorrow" true without code changes.
The runtime, UI, storage, and risk gate stay **asset-class-agnostic**; the differences live
here.

```rust
pub struct Instrument {
    pub instrument_id:   String,        // canonical id, e.g. "BTC-USDT", "AAPL"
    pub asset_class:     AssetClass,    // Crypto | Equity (Option/Bond/Etf/DexPool later)
    pub venue_id:        String,
    pub base_precision:  u32,           // size decimal places
    pub quote_precision: u32,           // price decimal places
    pub tick_size:       Decimal,
    pub lot_size:        Decimal,
    pub trading_hours:   TradingSchedule, // 24/7 for crypto; session+auctions for equities
    pub halt_behavior:   HaltPolicy,      // equities can halt; crypto generally doesn't
    pub trust_tier:      TrustTier,
    pub active:          bool,
}
```

Why this matters operationally: the freshness watchdog reads `trading_hours`/`halt_behavior` to
distinguish "market closed normally" (don't alarm) from "feed broke" (halt + alarm). The risk
gate reads `tick_size`/`lot_size` for fat-finger and validity checks. Adding options later =
extend `AssetClass`, add an options payload type, insert rows here.

## Timestamp model (load-bearing)

Every event carries up to four timestamps:

| Timestamp | Meaning |
|-----------|---------|
| `event_time` | When the source says the thing happened |
| `observed_time` | When our system first saw it |
| `ingested_time` | When it entered the bus |
| `available_time` | **When a strategy/backtest is allowed to use it** |

`available_time` is the most important field in the entire system. It is computed to include
processing delay (e.g. a feature that was only computable at `14:31:00.100` gets that as its
`available_time`, not `14:31:00.000`). It is the clock the replay engine sorts by, and it is what
prevents **lookahead bias** — the single most common reason backtests lie.

A 1-minute bar covering `10:30:00–10:30:59.999` does not become strategy-visible until
`10:31:00` plus watermark + processing delay. See watermark policy in
[03-data-engineering.md](./03-data-engineering.md).

## Event identity (timestamp is not enough)

Bad primary key: `timestamp`.

Deterministic identity (also the dedup key — derived from the source, never random at ingest):

```
lane + instrument_id + venue_id + sequence + source       (sequenced streams)
venue_id + exchange_trade_id                               (trades)
chain + tx_hash + log_index                                (on-chain, future)
```

For high-frequency streams, `sequence` numbers are mandatory for ordering and gap detection.

## Append-only, never rewrite

History is **never mutated**. Late data does not edit a published bar — it emits a **revision
event** (`revision > 0`) that supersedes the original. The original stays immutable. This is what
makes live and replay reproducible: the backtest can replay both the original and the revision at
their true `available_time`s and reproduce exactly what the strategy saw live, including whether
it saw a revision.
