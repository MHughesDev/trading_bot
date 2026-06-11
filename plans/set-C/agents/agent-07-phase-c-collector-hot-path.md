# Agent Query — Zero-Alloc Collector Normalization: xxh3 Identity + Borrowed WS Frames + Direct Decimal Conversion
## Covers Issues: #6, #7, #11
## Phase: C
## Estimated Effort: 3–5 days
## Prerequisites: #2 (intern table and xxhash-rust dependency must exist; xxh3 IDs feed into storage dedup in agent-09)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The three hottest allocation sources in the collector pipeline are: (1) UUID v5 computation via `format!("{venue}:{trade_id}")` plus SHA-1 hashing on every trade event; (2) owned `String` fields on every WS message struct that are heap-allocated by serde_json even though the WS frame buffer already contains the data; (3) `f64.to_string()` chains feeding into `Decimal::from_str()` where neither the intermediate string nor the Decimal conversion is necessary. Together these add hundreds of nanoseconds and 10+ heap allocations per trade event before the event even reaches the strategy bus.

## Codebase Context

- `crates/domain/src/ids.rs` (or `envelope.rs`) — around lines 37–58, contains `event_id` computation using `format!` + SHA-1 UUID v5 in `normalize()`.
- `crates/collectors/src/crypto/kraken.rs` — around lines 43–55, `KrakenTrade` struct owns `symbol: String, side: String, price: String, qty: String, timestamp: String`. Around line 108, the `normalize()` function calls the UUID computation. This is the canonical example of the pattern repeated across all collectors.
- `crates/collectors/src/equity/alpaca_data.rs` — around lines 42–60, same owned-String WS struct pattern.
- `crates/collectors/src/futures/tradovate.rs` — same pattern.
- `crates/collectors/src/options/tradier.rs` — around lines 93–176, same pattern plus `Decimal::from_str(&value.to_string())` chains.
- `crates/collectors/src/prediction/kalshi.rs` — around lines 97–114, `f64.to_string()` → `Decimal::from_str` pattern.
- `crates/collectors/src/fx/oanda.rs` — same pattern.
- `Cargo.toml` — `sonic_rs` is not yet a dependency; `xxhash-rust` may be added by agent-02.

The problematic UUID pattern (issue #6):
```rust
// normalize() in kraken.rs ~line 108 — runs on every trade
let id_str = format!("{}:{}", venue, trade.trade_id);   // ← heap alloc
let event_id = Uuid::new_v5(&NAMESPACE, id_str.as_bytes()); // ← SHA-1 hash
```

The problematic owned-String WS struct (issue #7):
```rust
// kraken.rs ~line 43 — serde allocates each field from the frame buffer
#[derive(serde::Deserialize)]
struct KrakenTrade {
    symbol: String,     // ← heap alloc from WS frame
    side: String,       // ← heap alloc
    price: String,      // ← heap alloc
    qty: String,        // ← heap alloc
    timestamp: String,  // ← heap alloc
}
```

The problematic Decimal conversion (issue #11):
```rust
// tradier.rs ~line 97 — allocates a String just to immediately parse it
let price = Decimal::from_str(&json_value.to_string())?;  // ← to_string() alloc
```

## Task

### Fix #6 — Remove UUID computation from collector normalize()

**Problem:** Every `normalize()` call in every collector computes `format!("{venue}:{trade_id}")` (heap alloc) then SHA-1 hashes it to produce a UUID v5. This runs once per trade event, on the hot path.

**Solution:** Remove the `event_id` computation from all collector `normalize()` functions. Move it to the storage writer boundary where it runs once per batch, not once per event. At the collector level, trade payloads carry a raw `trade_id: u64` (the venue's own ID) without any hashing.

**Implementation steps:**

1. Verify `xxhash-rust = { version = "0.8", features = ["xxh3"] }` is in workspace `Cargo.toml` (added by agent-02; add here if not present).

2. In `crates/domain/src/ids.rs`, remove the `compute_event_id(venue: &str, trade_id: &str) -> Uuid` function (or rename it `compute_storage_id` and move it to `crates/storage/src/dedup.rs`).

3. Remove the `event_id` field from `TradePayload` if it is set in `normalize()`. Instead, add a raw `venue_trade_id: u64` field that collectors populate directly from the parsed JSON without any hashing.

4. In `crates/storage/src/writer.rs` (the storage batch writer), compute the deduplication ID at flush time using xxh3:
   ```rust
   // Pack fields into a fixed-size array — no allocation
   let mut key = [0u8; 12];
   key[0..4].copy_from_slice(&venue_id.0.to_le_bytes());
   key[4..12].copy_from_slice(&venue_trade_id.to_le_bytes());
   let hash_id = xxhash_rust::xxh3::xxh3_128(&key);
   let storage_id = Uuid::from_u128(hash_id);
   ```
   This is called once per row in the flush batch, not once per incoming event.

5. Add a migration note comment in `crates/domain/src/ids.rs` documenting that historical IDs computed via SHA-1 UUID v5 will differ from the new xxh3-based IDs.

6. Remove `uuid` SHA-1 feature from `Cargo.toml` if it is no longer needed: `uuid = { version = "1", features = ["v4", "v5"] }` → `uuid = { version = "1", features = ["v4"] }`.

### Fix #7 — Borrowed &str fields in WS message structs

**Problem:** All collector WS message structs own their string fields (`String`), causing serde_json to allocate each field from the WS frame buffer on deserialization. The frame buffer already contains the string data — we should borrow it.

**Solution:** Use `sonic_rs` (SIMD JSON parser) with `#[serde(borrow)]` attributes to deserialize WS messages as `&'a str` slices into the frame buffer. Zero allocations for string fields.

**Implementation steps:**

1. Add `sonic_rs = "0.3"` to workspace `Cargo.toml`.

2. In `crates/collectors/src/crypto/kraken.rs`, change the `KrakenTrade` struct:
   ```rust
   #[derive(serde::Deserialize)]
   struct KrakenTrade<'a> {
       #[serde(borrow)]
       symbol: &'a str,
       #[serde(borrow)]
       side: &'a str,
       #[serde(borrow)]
       price: &'a str,    // JSON string field
       #[serde(borrow)]
       qty: &'a str,
       #[serde(borrow)]
       timestamp: &'a str,
   }
   ```

3. Change the deserialization call from `serde_json::from_slice` to `sonic_rs::from_slice`:
   ```rust
   // Before:
   let trades: Vec<KrakenTrade> = serde_json::from_slice(&frame_bytes)?;
   // After:
   let trades: Vec<KrakenTrade> = sonic_rs::from_slice(&frame_bytes)?;
   ```
   The frame buffer (`frame_bytes: &[u8]`) must live long enough for the borrowed `&str` references. In an async WS reader, this is typically the duration of one iteration of the receive loop — borrow `frame_bytes` for the duration of the `normalize()` call, then release.

4. In `normalize()`, parse `Decimal` directly from the borrowed slice:
   ```rust
   // Before (allocates intermediate String):
   let price = price_str.parse::<f64>().map(|v| Decimal::try_from(v))?;
   // After (no intermediate allocation):
   let price = Decimal::from_str(price_str)?;   // price_str is &'a str from frame
   ```
   `Decimal::from_str` accepts `&str` and does not allocate an intermediate String.

5. Apply the same `<'a>` lifetime pattern to all other collector WS message structs:
   - `crates/collectors/src/equity/alpaca_data.rs` — `AlpacaTrade<'a>`, `AlpacaQuote<'a>`
   - `crates/collectors/src/futures/tradovate.rs` — all `Tradovate*<'a>` message structs
   - `crates/collectors/src/options/tradier.rs` — all `Tradier*<'a>` message structs
   - `crates/collectors/src/prediction/kalshi.rs` — all `Kalshi*<'a>` message structs
   - `crates/collectors/src/fx/oanda.rs` — all `Oanda*<'a>` message structs
   For any field that is a JSON number (not a JSON string), keep it as `f64` in the struct — serde handles `f64` without allocation regardless.

### Fix #11 — Eliminate f64→string→Decimal chains

**Problem:** In `crates/collectors/src/prediction/kalshi.rs` (lines 97–114), `crates/collectors/src/futures/tradovate.rs` (lines 99–142), and `crates/collectors/src/options/tradier.rs` (lines 93–176): patterns like `Decimal::from_str(&value.to_string())` allocate a `String` just to parse it back as `Decimal`. This is unnecessary for both number-typed and string-typed JSON fields.

**Solution:**
- For JSON fields that are numbers (`f64` from serde): use `Decimal::try_from(f64_value)` — no intermediate string.
- For JSON fields that are strings (borrowed `&str` after fix #7): use `Decimal::from_str(str_slice)` — the slice is already a string, no conversion needed.
- For fields that only participate in arithmetic (quantities, not prices), keep as `f64` — no Decimal needed.

**Implementation steps:**

1. For each `Decimal::from_str(&value.to_string())` pattern found:
   - Determine whether `value` is a JSON number (`f64`) or JSON string (`&str`).
   - If JSON number: `Decimal::try_from(value)?` (delete `.to_string()` intermediate).
   - If JSON string (after fix #7): `Decimal::from_str(value)?` (already a `&str`, no change needed to the Decimal call, but the intermediate String from `to_string()` is gone because `value` is now `&str` from the borrowed struct).

2. Audit these specific patterns in each file:
   - `kalshi.rs:97-114` — identify each `f64.to_string()` and replace.
   - `tradovate.rs:99-142` — same.
   - `tradier.rs:93-176` — same.

3. For quantity fields that are only used in `qty * price` arithmetic and never stored in a `Money` type: keep as `f64`, remove any `Decimal` conversion entirely.

**Acceptance test:**
- Write a unit test for each collector's `normalize()` function that calls it with a synthetic WS frame (a `&[u8]` slice) and verifies the output `TradePayload` matches expected field values.
- Use a `#[global_allocator]` counter to verify that `normalize()` performs zero heap allocations for string fields (borrowed from the frame).
- `cargo test` must pass for all collector crates.

## Overall Acceptance Criteria
- [ ] Zero `format!` calls and zero SHA-1 UUID computation in any collector `normalize()` function
- [ ] Zero owned `String` fields on WS message structs across all 6 collectors (Kraken, Alpaca, Tradovate, Tradier, Kalshi, OANDA)
- [ ] Zero `f64.to_string()` followed by `Decimal::from_str` chains in any collector hot path
- [ ] `sonic_rs` added to workspace `Cargo.toml` and used for WS frame deserialization
- [ ] `normalize()` output is content-equivalent to the previous output (round-trip unit tests pass)
- [ ] Storage writer computes xxh3 dedup IDs at flush time, not per event in collectors
- [ ] `cargo build --release` succeeds for all collector crates

## Files to Touch
- `crates/domain/src/ids.rs` — remove SHA-1 UUID computation; add migration note; move storage ID computation to storage crate
- `crates/collectors/src/crypto/kraken.rs` — lifetime struct; sonic_rs; remove UUID; fix Decimal
- `crates/collectors/src/equity/alpaca_data.rs` — same pattern
- `crates/collectors/src/futures/tradovate.rs` — same pattern
- `crates/collectors/src/options/tradier.rs` — same pattern; eliminate f64→string→Decimal chains
- `crates/collectors/src/prediction/kalshi.rs` — same pattern; eliminate f64→string→Decimal chains
- `crates/collectors/src/fx/oanda.rs` — same pattern
- `crates/storage/src/writer.rs` — add xxh3 ID computation at flush time
- `Cargo.toml` — add `sonic_rs = "0.3"`; update uuid features if v5 no longer needed
