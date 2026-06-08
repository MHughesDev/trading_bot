# Add a Venue

> Checklist validated by Phase 6: adding Alpaca equity data (the second venue/asset class).  
> Phase 7 / P7-T05.

A "venue" is a market data source (e.g., Kraken, Alpaca IEX) or an execution venue (e.g., Coinbase, Alpaca paper). This procedure covers adding a new **market data venue** (collector + normalized events). Adding a new execution venue follows the same pattern for the adapter but does not need a new satellite process.

---

## Prerequisites

Read before starting:
- `crates/domain/src/payloads/` — existing payload types (trade, quote, orderbook, bar)
- `crates/collectors/src/` — existing collector implementations (Kraken, Alpaca)
- `docs/specs/COMP-001-data-quality-and-ingestion.md` — ingestion contract

---

## Checklist

### 1. Domain: instrument metadata rows

- [ ] Add the new venue's instruments to `crates/storage/src/postgres/instruments.rs` as seed data (follow the `equity_seed_instruments()` pattern).
- [ ] For each instrument, set:
  - `asset_class` (`AssetClass::CryptoSpotCex`, `AssetClass::Equity`, etc.)
  - `venue_id` (e.g., `"alpaca"`, `"kraken"`)
  - `tick_size`, `lot_size`, `base_precision`, `quote_precision`
  - `trading_hours` (`TradingSchedule::always_open()` for 24/7; a named session for session-bound instruments)
  - `halt_behavior` (`HaltPolicy::NonHaltable` for crypto; `HaltPolicy::Haltable` for equities)
  - `trust_tier` (`TrustTier::CentralizedExchange` for crypto; `TrustTier::Regulated` for equities)
  - `watermark_secs` (2 for liquid CEX; tune per venue)
- [ ] Write a Postgres migration in `migrations/` to INSERT the new rows.
- [ ] Add unit tests in `instruments.rs` verifying the metadata is correct (follow `equity_seeds_have_correct_metadata` pattern).

### 2. Payload: normalize raw venue messages

- [ ] Study the venue's API/WS protocol documentation.
- [ ] Create `crates/collectors/src/<asset_class>/<venue>.rs`.
- [ ] Define a `struct <Venue>Message` (or similar) with `#[derive(Deserialize)]` matching the venue's wire format.
- [ ] Implement `normalize(&self, msg: &<Venue>Message, raw: &[u8], seq: u64) -> Result<EventEnvelope<TradePayload>, NormalizeError>`:
  - Parse price/size as `f64`, convert to `Decimal` via `Decimal::from_str(&f.to_string())` — never use `f64` in domain types directly.
  - Set `TrustTier` appropriately for the asset class.
  - Set `side` (`TradeSide::Unknown` if the venue doesn't report it).
  - Derive the `exchange_trade_id` from the venue's trade/sequence identifier; fall back to `seq.to_string()` if absent.
- [ ] Write unit tests for `normalize()`:
  - Happy path with all fields present.
  - Missing required field returns `NormalizeError::MissingField`.
  - Trust tier is correct for the asset class.

### 3. Collector: connect, auth, subscribe, reconnect

- [ ] Implement `<Venue>Collector` struct implementing `crate::Collector`.
- [ ] In `run()`:
  - Read credentials from env vars (do not hard-code).
  - Use `ReconnectPolicy` for all connect/auth/subscribe failures.
  - Use `GapDetector` to warn on sequence gaps.
  - Call `quarantine_or_publish` (from `crate::normalizer`) on each message result.
  - Handle Ping/Pong and Close frames for WebSocket venues.
- [ ] Register the new collector in `crates/collectors/src/<asset_class>/mod.rs` and in `crates/collectors/src/lib.rs`.

### 4. Satellite binary (for data sources)

- [ ] Create `apps/collector-<venue>/` with a `Cargo.toml` and `src/main.rs`.
- [ ] Follow the `apps/collector-equity/` pattern:
  - Load config via `cfg::load()`.
  - Init observability via `observability::init()`.
  - Connect to NATS via `event_bus::connect()` + `setup_streams()`.
  - Parse symbol(s) from CLI args.
  - Spawn one tokio task per symbol.
- [ ] Add the new app to the workspace `Cargo.toml` `[workspace.members]`.
- [ ] Add the new binary to `.github/workflows/release.yml` (build step) and `Dockerfile` (COPY step).

### 5. Execution adapter (if the venue also executes orders)

- [ ] Implement `Broker` trait in `crates/execution/src/<venue>.rs` (follow `alpaca.rs` or `coinbase.rs`).
- [ ] Register in `crates/execution/src/lib.rs`.
- [ ] Add to the `match` in `crates/api/src/routes/orders.rs` if the venue needs a new rejection variant in `RiskRejection`.

### 6. Risk gate: new rejection variants (if needed)

- [ ] If the venue introduces new rejection reasons not covered by existing `RiskRejection` variants:
  - Add the variant to `crates/domain/src/error.rs`.
  - Add the limit check to `crates/risk/src/limits.rs`.
  - Wire the check into `RiskGate::run_checks()` in `crates/risk/src/gate.rs`.
  - Handle the new variant in `risk_rejection_response()` in `crates/api/src/routes/orders.rs`.
  - Add adversarial tests in `crates/risk/tests/`.

### 7. Integration tests

- [ ] Add cross-asset parity tests following the `crates/risk/tests/cross_asset_parity.rs` pattern — prove the new venue works through the same gate as existing venues.
- [ ] Run `cargo test --workspace` and confirm all tests pass.

### 8. Documentation

- [ ] Update `docs/architecture.md`:
  - Add the new component row to the Components table.
  - Add the new app to the repo structure listing.
  - Update the data flow diagram if the new venue has a different entry point.
- [ ] Add instrument seed data to `docs/specs/DATA-002-instrument-metadata.md` acceptance criteria evidence.

---

## What you must NOT do

- **Do not use `f64` in domain types.** All prices and sizes must go through `Decimal`.
- **Do not branch on `AssetClass` in core code.** Differences must live in instrument metadata properties (`halt_policy`, `trading_hours`, `trust_tier`).
- **Do not submit orders that bypass the risk gate.** The `ApprovedOrder` sealed type (`_sealed: ()`) enforces this at compile time — if you find yourself working around it, stop.
- **Do not read the wall clock in the strategy runtime.** Use `world.now()` which returns `event.available_time`.
