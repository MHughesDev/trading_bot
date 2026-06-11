# Issue #011 — Double conversion: f64→string→Decimal→string

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | C |
| Pattern | Allocation |
| Quick Win | No |
| Latency Impact | 5–10 allocs + 2–3 parses per field |
| Location | `crates/collectors/src/prediction/kalshi.rs:97-114` |

## Problem
Numeric fields in the Kalshi collector are converted via `f64.to_string()` then `Decimal::from_str(...)` then `.to_string()` — a triple conversion with 2-3 heap allocations per numeric field. This appears in the normalization path called for every Kalshi event.

## Root Cause
The Kalshi API returns JSON numbers as floats. The original code converts them to String first as a workaround for Decimal parsing, then converts to string again for inclusion in the payload. The intermediate string is unnecessary.

## Implementation Plan
### Step 1 — Replace f64→string→Decimal with Decimal::try_from(f64)
Replace the pattern:
```rust
Decimal::from_str(&f64_value.to_string())
```
with:
```rust
Decimal::try_from(f64_value)
```
This constructs Decimal directly from the float value with no heap allocation.

### Step 2 — Identify non-money fields that don't need Decimal at all
Audit all numeric fields in the Kalshi payload. Fields representing raw scores, probabilities, or non-monetary values can remain as `f64` throughout — only price/amount fields need `Decimal`.

### Step 3 — Apply to Tradovate and Tradier collectors
The same f64→String→Decimal→String anti-pattern appears in `crates/collectors/src/futures/tradovate.rs` and `crates/collectors/src/options/tradier.rs`. Apply the same fix.

### Step 4 — Add a test for Decimal precision
Verify that `Decimal::try_from(f64_price)` produces the correct value for representative Kalshi prices (probabilities between 0 and 1). Document precision limits.

## Acceptance Criteria
- [ ] Zero intermediate string allocations in numeric field conversion in Kalshi normalizer
- [ ] Tradovate and Tradier collectors use same pattern
- [ ] Non-money numeric fields use `f64` directly; only price/amount use `Decimal`
- [ ] Unit test covering Decimal precision for Kalshi probability values

## Files to Change
- `crates/collectors/src/prediction/kalshi.rs` — replace triple conversion at lines 97-114
- `crates/collectors/src/futures/tradovate.rs` — same fix
- `crates/collectors/src/options/tradier.rs` — same fix
