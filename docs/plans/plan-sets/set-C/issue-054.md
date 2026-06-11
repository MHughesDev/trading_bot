# Issue #054 — Order intent: strategy_id cloned unnecessarily

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 string clone per order intent |
| Location | `crates/strategy-runtime/src/intents.rs:30` |

## Problem
`Some(strategy_id.to_owned())` clones the strategy ID string for every order intent, even though the caller already owns it. Order intents are generated every time a strategy decides to trade — potentially hundreds per second under active trading.

## Root Cause
The order intent construction at `intents.rs:30` takes `strategy_id: &str` and calls `.to_owned()` to create an owned `Option<String>`. The caller owns the strategy ID but passes it as a reference.

## Implementation Plan
### Step 1 — Change strategy_id to Arc<str>
If strategy IDs are stable (assigned at instance creation and never changed), store them as `Arc<str>` on the `StrategyInstance`. Passing the Arc into the intent is a cheap atomic increment:
```rust
struct OrderIntent {
    strategy_id: Option<Arc<str>>,
    // ...
}
fn build_intent(strategy_id: &Arc<str>) -> OrderIntent {
    OrderIntent {
        strategy_id: Some(Arc::clone(strategy_id)),
        // ...
    }
}
```

### Step 2 — Alternative: accept Option<&str> and defer to_owned
Change the build function to accept `Option<&str>` and call `.to_owned()` only at serialization time (when sending to the broker API). This keeps the hot path allocation-free.

### Step 3 — Coordinate with #2
If strategy IDs are interned as `StrategyId(u32)` as part of the broader #2 interning effort, use the u32 ID throughout — no string needed.

## Acceptance Criteria
- [ ] No `to_owned()` call on strategy_id at `intents.rs:30` per order intent
- [ ] strategy_id stored as Arc<str> or StrategyId(u32)
- [ ] Order intent construction: zero String allocation for strategy_id
- [ ] Order execution tests pass with new strategy_id type

## Files to Change
- `crates/strategy-runtime/src/intents.rs` — replace to_owned() at line 30 with Arc::clone or u32 ID
