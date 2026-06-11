# Issue #016 — Rollup: multiple HashMap rebuilds per request

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Data structure |
| Quick Win | Yes |
| Latency Impact | 4 HashMaps + 3–4 iterations per rollup request |
| Location | `crates/api/src/rollup/mod.rs:72-89` |

## Problem
Multiple HashMaps are built and iterated to group rollup data; the same data is iterated 3–4 times. While rollup is not a hot path (it's user-initiated), it represents unnecessary CPU and memory churn that adds latency to API responses.

## Root Cause
The rollup function at `mod.rs:72-89` builds intermediate HashMaps to group and aggregate data. These intermediate structures could be replaced with a single-pass grouping algorithm, especially since the output groups are known ahead of time.

## Implementation Plan
### Step 1 — Profile the rollup path
Identify which of the 4 HashMaps is the dominant cost. Use criterion or flamegraph on a representative rollup request.

### Step 2 — Replace multi-pass with single-pass grouping
Redesign the grouping logic to iterate the source data once:
```rust
let mut groups: HashMap<GroupKey, RollupAccumulator> = HashMap::with_capacity(expected_groups);
for item in source_data {
    groups.entry(item.key()).or_default().accumulate(item);
}
```
Use `HashMap::with_capacity(n)` where `n` is the expected number of groups.

### Step 3 — Use borrowed keys where possible
Use `Rc<&T>` or `&str` as HashMap keys during the grouping phase to avoid cloning keys into every intermediate map.

### Step 4 — Pre-sort if grouping by sorted key
If the rollup groups are sorted (e.g., by date or instrument), sort once and use group_by iteration (no HashMap needed).

## Acceptance Criteria
- [ ] Rollup request uses at most 1 HashMap for grouping (not 4)
- [ ] Single pass over source data (not 3–4 iterations)
- [ ] `HashMap::with_capacity` used where size is predictable
- [ ] API response time for rollup requests does not regress

## Files to Change
- `crates/api/src/rollup/mod.rs` — replace multi-pass HashMap rebuilds at lines 72-89 with single-pass grouping
