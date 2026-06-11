# Issue #033 — Account source: .to_owned() per JSON field

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 2–3 per position/fill |
| Location | `crates/execution/src/account/kalshi.rs:115` |

## Problem
`.to_owned()` is called on JSON string fields that could be borrowed from the parsed response. Each position or fill record from the account API allocates 2-3 Strings for fields that are only needed to construct a domain struct and are then dropped.

## Root Cause
The response deserialization structs use owned `String` fields. `serde_json` allocates each from the response body even though the response body lives long enough to borrow from.

## Implementation Plan
### Step 1 — Add #[serde(borrow)] to response structs
Change the Kalshi account response deserialization struct to use borrowed fields:
```rust
#[derive(Deserialize)]
struct KalshiPosition<'a> {
    #[serde(borrow)]
    ticker: &'a str,
    #[serde(borrow)]
    side: &'a str,
    quantity: i64,
}
```

### Step 2 — Pre-allocate Vec with capacity
Before collecting positions/fills, call `Vec::with_capacity(positions.len())` to avoid Vec reallocations.

### Step 3 — Remove .to_owned() at line 115
With borrowed fields, `.to_owned()` at line 115 is no longer needed — the borrowed &str can be passed directly to the domain struct constructor, which should also accept &str (or intern to u32).

### Step 4 — Apply to all account response structs
Check Alpaca, Kraken, and Oanda account response structs for the same pattern.

## Acceptance Criteria
- [ ] No `.to_owned()` on JSON string fields in Kalshi account response processing
- [ ] Response struct uses `#[serde(borrow)]` and `&'a str` fields
- [ ] `Vec::with_capacity` used for position/fill collections
- [ ] Account sync tests pass with borrowed deserialization

## Files to Change
- `crates/execution/src/account/kalshi.rs` — add serde(borrow); remove .to_owned() at line 115; add with_capacity
