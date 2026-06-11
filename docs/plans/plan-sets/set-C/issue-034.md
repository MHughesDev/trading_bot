# Issue #034 — Venue router: triple string clone on key

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | D |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 3 clones per collector start/stop |
| Location | `crates/venue-router/src/lifecycle.rs:42` |

## Problem
Three string fields are cloned to build a registry key on every collector lifecycle event (start or stop). While lifecycle events are infrequent, this is needless allocation for a composite key that could be a struct of u32 IDs.

## Root Cause
The venue-router builds a registry key by cloning three string fields (venue, instrument, lane or similar) to construct a composite String or tuple of Strings. This is used as a HashMap key for the collector registry.

## Implementation Plan
### Step 1 — Define a composite key struct
```rust
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
struct CollectorKey {
    venue: VenueId,       // u32
    instrument: InstrumentId, // u32
    lane: LaneId,         // u16 or u32
}
```
All fields are Copy integers — no allocation needed.

### Step 2 — Pass the key struct instead of cloning strings
At `lifecycle.rs:42`, construct the `CollectorKey` from the pre-interned IDs (from #2). Pass to the registry by value (it's Copy).

### Step 3 — Update the registry key type
Change `HashMap<String, CollectorState>` to `HashMap<CollectorKey, CollectorState>`. Remove all string concatenation / cloning for key construction.

### Step 4 — Consolidate with #42 (to_owned on stop path)
Issue #42 is the same problem on the stop path. Fix both in the same PR.

## Acceptance Criteria
- [ ] Zero String allocations for collector registry key construction
- [ ] CollectorKey is a Copy struct of u32 IDs
- [ ] Collector start and stop: zero clone calls at `lifecycle.rs:42`
- [ ] Venue router lifecycle tests pass

## Files to Change
- `crates/venue-router/src/lifecycle.rs` — replace triple String clone at line 42 with CollectorKey(u32, u32, u32) construction
