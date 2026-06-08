# DATA-002: Instrument Metadata

**Status:** Draft
**Version:** 0.1
**ADR(s):** ADR-0001
**Success Conditions:** SC-5

## 1. Purpose

Defines the `Instrument` struct and its associated types — `AssetClass`, `TradingSchedule`, `HaltPolicy`, and `TrustTier` — that make the entire platform asset-class-agnostic. All per-asset-class differences (trading hours, precision, halt behavior, tick/lot constraints) live in this metadata table so the runtime, risk gate, UI, and storage layers never branch on asset class. Adding a new asset class is an additive change: a new collector, new payload type(s), and new rows here — zero core code changes.

## 2. Scope & Non-Goals

**In scope:**
- `Instrument` struct definition and field semantics.
- `AssetClass` enum values (v1 plus intended future expansion).
- `TradingSchedule` struct and session-vs-24/7 semantics.
- `HaltPolicy` enum and its effect on the freshness watchdog and risk gate.
- `TrustTier` enum (values shared with `EventEnvelope` — see DATA-001).
- Rationale for why this design makes asset-class expansion additive.

**Not in scope (deliberate):**
- Order routing rules per venue — owned by the execution engine broker adapter (COMP-002).
- Fee/commission schedules — deferred post-v1.
- Corporate actions (splits, dividends) — deferred post-v1.
- Full market microstructure for options, futures, or DEX assets — additive extensions per class.

## 3. Design

### 3.1 Instrument Struct

```rust
pub struct Instrument {
    pub instrument_id:   String,           // canonical id, e.g. "BTC-USDT", "AAPL"
    pub asset_class:     AssetClass,       // drives validation and lane selection
    pub venue_id:        String,           // e.g. "coinbase", "alpaca"
    pub base_precision:  u32,              // size decimal places (e.g. 8 for BTC)
    pub quote_precision: u32,              // price decimal places
    pub tick_size:       Decimal,          // minimum price increment
    pub lot_size:        Decimal,          // minimum size increment
    pub trading_hours:   TradingSchedule,  // 24/7 for crypto; session+auctions for equities
    pub halt_behavior:   HaltPolicy,       // equities can halt; crypto generally doesn't
    pub trust_tier:      TrustTier,        // matches trust_tier on EventEnvelope
    pub active:          bool,             // false = delisted / inactive
}
```

### 3.2 AssetClass Enum

The v1 values cover Coinbase and Alpaca. The enum is deliberately open for extension — the runtime and risk gate never match exhaustively on it.

```rust
pub enum AssetClass {
    // v1
    CryptoSpotCex,   // e.g. BTC-USDT on Coinbase
    Equity,          // common stocks, REITs, ADRs on Alpaca

    // planned — no core changes required to add these
    Etf,
    CryptoSpotDex,   // Uniswap, Curve
    FuturesExpiring,
    PerpetualSwap,
    Option,
    Bond,
    Fx,
    Nft,
    PredictionMarket,
}
```

### 3.3 TradingSchedule

Encodes when a market is open for trading. The freshness watchdog reads this to distinguish "market closed normally" (do not alarm) from "feed broke" (halt and alarm).

```rust
pub struct TradingSchedule {
    pub timezone:         String,          // IANA tz, e.g. "America/New_York"
    pub sessions:         Vec<Session>,    // empty = 24/7 (crypto)
    pub has_pre_market:   bool,
    pub has_post_market:  bool,
}

pub struct Session {
    pub weekday: Weekday,                  // Mon–Fri for US equities
    pub open:    NaiveTime,                // e.g. 09:30
    pub close:   NaiveTime,               // e.g. 16:00
}
```

Crypto instruments have `sessions: vec![]` signalling 24/7 operation — the watchdog treats any quiet period as suspect. Equity instruments list explicit sessions; quietness during off-hours is normal.

### 3.4 HaltPolicy

```rust
pub enum HaltPolicy {
    /// Asset can be halted by exchange/regulator; halt state must be
    /// respected by the risk gate and freshness watchdog.
    Haltable,

    /// Asset does not halt under normal operation (most crypto spot).
    /// Unexpected silence always triggers a feed-stale alarm.
    NonHaltable,
}
```

### 3.5 TrustTier

`TrustTier` is a shared enum between `Instrument` and `EventEnvelope<T>`. It is defined here as part of the metadata model and referenced by DATA-001.

```rust
pub enum TrustTier {
    Regulated,             // stocks, ETFs, bonds
    CentralizedExchange,   // Binance, Coinbase
    OnchainConfirmed,      // confirmed swaps (future)
    OnchainTentative,      // sub-confirmation swaps (future)
    SocialDerived,         // sentiment signals (future)
}
```

### 3.6 Why This Design Makes Expansion Additive

The risk gate reads `tick_size` and `lot_size` for fat-finger and validity checks. The freshness watchdog reads `trading_hours` and `halt_behavior`. The strategy validator reads `asset_class` to reject incompatible strategy initialization. The storage writers use `base_precision` / `quote_precision` to quantize `Decimal` values.

None of these components branch on specific asset classes — they branch on the *properties* (`HaltPolicy`, `TradingSchedule`, tick/lot values) that the metadata table expresses. Adding options tomorrow means:
1. Add `AssetClass::Option` (non-breaking enum extension).
2. Define the options-specific payload type in `DATA-001`.
3. Write an options collector.
4. Insert instrument rows with appropriate `TradingSchedule`, `HaltPolicy`, precision, etc.

Zero changes to the runtime, risk gate, storage writers, or replay engine.

## 4. Interfaces

**Stored in:** Postgres (`instruments` table) — transactional, not high-volume.

**Cached in:** Redis, keyed `instrument:{instrument_id}` — read by the risk gate and freshness watchdog on every order and every feed-quiet event. TTL matches expected update frequency (hours).

**Read by:**
- Risk gate — `tick_size`, `lot_size`, `halt_behavior`, `active` (COMP-002).
- Freshness watchdog — `trading_hours`, `halt_behavior` (COMP-001).
- Strategy validator — `asset_class`, `trust_tier` (FEAT-001).
- Normalization layer — `base_precision`, `quote_precision` for `Decimal` quantization (COMP-001).
- UI / instrument detail view — full struct for display.

**Written by:**
- Admin / seed scripts at startup.
- Collector processes on venue metadata change (e.g. tick-size change).

**REST endpoints:**
```
GET  /api/instruments          — list all active instruments
GET  /api/instruments/{id}     — single instrument by canonical id
GET  /api/assets               — grouped by asset_class
```

## 5. Dependencies

- `rust_decimal` crate — for `Decimal` fields (`tick_size`, `lot_size`).
- `chrono` crate — for `NaiveTime`, `Weekday` in `TradingSchedule`.
- DATA-001 — `TrustTier` is shared; `instrument_id` FK from `EventEnvelope`.
- COMP-001 — freshness watchdog reads `trading_hours` / `halt_behavior`.
- COMP-002 — risk gate reads `tick_size`, `lot_size`, `halt_behavior`.
- FEAT-001 — strategy validator reads `asset_class` on instance initialization.

## 6. Acceptance Criteria

- [ ] AC-1: A strategy definition with `asset_class: "crypto_spot_cex"` cannot be initialized on an instrument where `Instrument.asset_class == AssetClass::Equity` — the validator returns a structured rejection — Verified by: [—]
- [ ] AC-2: The freshness watchdog for an equity instrument does not raise a stale-feed alarm when the instrument's `TradingSchedule` shows the market is closed — Verified by: [—]
- [ ] AC-3: The freshness watchdog for a crypto instrument (empty `sessions`) raises a stale-feed alarm whenever the configured quiet-period threshold is exceeded, regardless of time of day — Verified by: [—]
- [ ] AC-4: The risk gate rejects an order whose `size` is not a multiple of `lot_size` for the bound instrument — Verified by: [—]
- [ ] AC-5: The risk gate rejects an order whose `price` is not a multiple of `tick_size` for the bound instrument — Verified by: [—]
- [ ] AC-6: Adding a new `AssetClass` variant and inserting instrument rows for it does not require any change to the risk gate, strategy runtime, storage writers, or replay engine crates — Verified by: [—]

## 7. Open Questions

None at this revision. Future asset classes (options greeks, AMM pricing mechanics) will extend this model additively when those collectors are built.
