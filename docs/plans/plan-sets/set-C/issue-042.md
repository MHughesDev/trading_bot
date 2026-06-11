# Issue #042 — Venue-router: to_owned on parameters

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 3 String allocs per stop |
| Location | `crates/venue-router/src/lifecycle.rs:78-80` |

## Problem
Three `.to_owned()` calls on parameters that are immediately used to construct a key, then discarded. This mirrors the start path issue (#34) but occurs on the stop path. Three String allocations for a key that could be a struct of Copy integers.

## Root Cause
Same root cause as #34: the stop function accepts `&str` parameters and calls `.to_owned()` on each to build a HashMap key, when the key should be a `CollectorKey(u32, u32, u32)` struct constructed from pre-interned IDs.

## Implementation Plan
### Step 1 — Coordinate with #34
If #34 has introduced `CollectorKey` as a Copy struct of u32 IDs, apply the same fix here. The stop function should accept `CollectorKey` directly (or the three u32 IDs) rather than three `&str` parameters.

### Step 2 — Remove .to_owned() at lines 78-80
Replace:
```rust
fn stop_collector(&self, venue: &str, instrument: &str, lane: &str) {
    let key = (venue.to_owned(), instrument.to_owned(), lane.to_owned()); // ← 3 allocs
    self.registry.remove(&key);
}
```
with:
```rust
fn stop_collector(&self, key: CollectorKey) {
    self.registry.remove(&key); // ← 0 allocs
}
```

### Step 3 — Update all call sites
Pass pre-constructed `CollectorKey` from the callers instead of passing three separate string references.

## Acceptance Criteria
- [ ] Zero String allocations for key construction at `lifecycle.rs:78-80`
- [ ] Stop function uses CollectorKey or u32 IDs (consistent with #34)
- [ ] Collector stop test passes

## Files to Change
- `crates/venue-router/src/lifecycle.rs` — remove .to_owned() at lines 78-80; use CollectorKey struct (from #34)
