# Issue #058 — Graph: serde_json::to_value() per asset class

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Serialization |
| Quick Win | Yes |
| Latency Impact | 11 enum→JSON→str→clone conversions at init |
| Location | `crates/graph/src/populate.rs:80-85` |

## Problem
`serde_json::to_value(a)` is called on every asset class to convert the enum to a JSON string value, just to extract the string representation. With 11 asset classes, this is 11 JSON serialization cycles at graph initialization — overkill for what is essentially `AssetClass::as_str()`.

## Root Cause
The graph population code uses `serde_json::to_value()` to get the string name of an enum variant. This is a JSON serialization of an enum just to get its string representation — a much cheaper operation than JSON serialization.

## Implementation Plan
### Step 1 — Add an as_str() method to AssetClass
```rust
impl AssetClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Equity => "equity",
            Self::Crypto => "crypto",
            Self::Futures => "futures",
            Self::Options => "options",
            Self::Fx => "fx",
            Self::Prediction => "prediction",
            Self::Fixed => "fixed",
            Self::Commodity => "commodity",
            Self::Etf => "etf",
            Self::Index => "index",
            Self::Alternative => "alternative",
        }
    }
}
```
Returns `&'static str` — zero allocation.

### Step 2 — Replace serde_json::to_value(a) with a.as_str()
At `populate.rs:80-85`:
```rust
// Before:
let s = serde_json::to_value(asset_class).unwrap().as_str().unwrap().to_owned();
// After:
let s = asset_class.as_str(); // &'static str, no allocation
```

### Step 3 — Remove serde dependency from this code path
If `serde_json::to_value` was the only JSON operation in `populate.rs`, remove the serde_json import from this file.

## Acceptance Criteria
- [ ] No `serde_json::to_value()` call at `populate.rs:80-85`
- [ ] `AssetClass::as_str()` implemented with `&'static str` return
- [ ] Graph population test passes: all 11 asset classes registered correctly
- [ ] Zero allocations for asset class string representation at graph init

## Files to Change
- `crates/graph/src/populate.rs` — replace serde_json::to_value at lines 80-85 with as_str() method call
