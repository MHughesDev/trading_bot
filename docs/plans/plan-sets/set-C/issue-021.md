# Issue #021 — Universe entry: String+HashMap fields

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Data structure |
| Quick Win | No |
| Latency Impact | Re-hashing per rank/filter |
| Location | `crates/strategy-runtime/src/nodes/mod.rs:22-26` |

## Problem
Universe entries contain String fields and HashMap feature slots that are rebuilt on every filter/rank pass. Every time the pipeline evaluates, universe entries re-hash their feature data to respond to filter and rank queries. This adds O(feature_count) hashing per entry per pipeline pass.

## Root Cause
`UniverseEntry` (at lines 22-26) stores feature data as `HashMap<String, f64>` fields. Each filter/rank lookup hashes the feature name string to retrieve the value, and the HashMap is rebuilt rather than reused from a stable allocation.

## Implementation Plan
### Step 1 — Coordinate with #4 (slot-array features)
If #4 is landed, feature data in universe entries should also use slot arrays (`Vec<f64>`) indexed by the same `u16` slot IDs. This eliminates String keys from UniverseEntry entirely.

### Step 2 — Replace HashMap with SmallVec + interned feature IDs
Change `UniverseEntry.features` from `HashMap<String, f64>` to:
```rust
struct UniverseEntry {
    instrument: InstrumentId,       // interned u32
    features: SmallVec<[f64; 32]>, // indexed by slot ID from #4
}
```
`SmallVec<[f64; 32]>` avoids heap allocation for up to 32 features (stack-allocated inline array).

### Step 3 — Pre-sort entries at universe construction
Sort universe entries once at construction time (by instrument ID). Filter and rank passes can use binary search or index-based access on the sorted array rather than HashMap.

### Step 4 — Cache sorted universe between pipeline passes
Store the sorted `Vec<UniverseEntry>` on the instance and update incrementally (insert/remove) rather than rebuilding on every pipeline evaluation.

## Acceptance Criteria
- [ ] No HashMap in UniverseEntry for feature data
- [ ] SmallVec<[f64; 32]> or Vec<f64> indexed by slot ID
- [ ] Universe pre-sorted at construction; not rebuilt per tick
- [ ] Filter/rank passes use indexed access without hashing

## Files to Change
- `crates/strategy-runtime/src/nodes/mod.rs` — change UniverseEntry struct at lines 22-26; remove HashMap fields
