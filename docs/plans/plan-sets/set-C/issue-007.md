# Issue #007 — Collector deserializes into owned Strings per trade

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | C |
| Pattern | Allocation |
| Quick Win | No |
| Latency Impact | 5 owned String allocations per trade entry out of the WS frame |
| Location | `crates/collectors/src/crypto/kraken.rs:43-55` |

## Problem
`KrakenTrade` owns `symbol`, `side`, `price`, `qty`, `timestamp` as Strings. `serde_json` allocates each of these from the WebSocket frame buffer just to parse and immediately drop them when constructing a `TradePayload`. Five heap allocations per trade, per collector, for data that only lives for the duration of a single function call.

## Root Cause
The WS message structs were written with owned `String` fields for convenience. Since the deserialized struct is only used to construct the domain `TradePayload` and is then dropped, borrowing from the frame buffer with `&'a str` would be zero-copy.

## Implementation Plan
### Step 1 — Add sonic_rs to workspace
Add `sonic_rs` to `Cargo.toml` as a drop-in serde-compatible JSON parser that supports borrowing from the input buffer.

### Step 2 — Change owned String fields to borrowed &'a str
In `KrakenTrade` (and all equivalent collector WS-message structs), change:
```rust
struct KrakenTrade {
    symbol: String,
    side: String,
    price: String,
    qty: String,
    timestamp: String,
}
```
to:
```rust
#[derive(Deserialize)]
struct KrakenTrade<'a> {
    #[serde(borrow)]
    symbol: &'a str,
    #[serde(borrow)]
    side: &'a str,
    price: &'a str,
    qty: &'a str,
    timestamp: &'a str,
}
```

### Step 3 — Swap serde_json for sonic_rs
Replace `serde_json::from_str(&frame)` with `sonic_rs::from_str(&frame)` in the WS receive loop. sonic_rs supports zero-copy deserialization with `#[serde(borrow)]`.

### Step 4 — Parse numeric fields directly from &str
Use `price.parse::<f64>()` or `Decimal::from_str(price)` on the borrowed slice; avoid intermediate String.

### Step 5 — Apply to all collectors
Apply the same pattern to: `crates/collectors/src/equity/alpaca_data.rs`, `crates/collectors/src/futures/tradovate.rs`, `crates/collectors/src/options/tradier.rs`, `crates/collectors/src/prediction/kalshi.rs`, `crates/collectors/src/fx/oanda.rs`.

## Acceptance Criteria
- [ ] Zero owned Strings allocated between socket read and TradePayload construction in Kraken collector
- [ ] All listed collectors updated with borrowed &'a str fields and sonic_rs deserialization
- [ ] Collector unit tests pass with borrowed deserialization
- [ ] No lifetime errors at compile time

## Files to Change
- `crates/collectors/src/crypto/kraken.rs` — borrow WS message fields; switch to sonic_rs
- `crates/collectors/src/equity/alpaca_data.rs` — same pattern
- `crates/collectors/src/futures/tradovate.rs` — same pattern
- `crates/collectors/src/options/tradier.rs` — same pattern
- `crates/collectors/src/prediction/kalshi.rs` — same pattern
- `crates/collectors/src/fx/oanda.rs` — same pattern
