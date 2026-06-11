# Issue #017 — FeatureValue name cloned as HashMap key

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Clone |
| Quick Win | No |
| Latency Impact | 1 name-clone per feature event |
| Location | `crates/strategy-runtime/src/world.rs:100` |

## Problem
Feature name String is cloned on every insert into the feature HashMap. For a strategy with 50 features receiving 1000 ticks/sec, this is 50,000 String allocations per second just for HashMap keys.

## Root Cause
`HashMap<String, FeatureValue>` requires ownership of the key. When inserting a feature value, the feature name (a &str or borrowed value from the event) must be cloned to a String to satisfy the HashMap's ownership requirement.

## Note on Consolidation
This issue is fully resolved by Issue #4 (slot-array features). When #4 lands, String keys are replaced by `u16` slot indices — no clone needed. This file is kept for tracking purposes.

If implementing before #4: use `Arc<str>` as the HashMap key type so that cloning is a cheap atomic increment rather than a heap allocation.

## Implementation Plan
### Step 1 — Coordinate with #4
Verify that after #4 lands, `world.rs:100` has no String clone for HashMap key insertion. Mark this issue as resolved by #4.

### Step 2 — Interim fix (if #4 is not yet landed)
Change `HashMap<String, FeatureValue>` to `HashMap<Arc<str>, FeatureValue>`. Change insert sites to clone the `Arc<str>` (atomic increment) rather than the String (heap allocation):
```rust
self.features.insert(Arc::clone(&feature.name), feature.value);
```
Feature names become `Arc<str>` at the point they are first created (e.g., at strategy load time).

## Acceptance Criteria
- [ ] No String heap allocation for feature HashMap key insertion per tick
- [ ] If #4 is landed: HashMap replaced by Vec<f64> slot array; this issue is moot
- [ ] If interim fix: `Arc<str>` keys; clone cost is atomic increment only

## Files to Change
- `crates/strategy-runtime/src/world.rs` — change HashMap key type to Arc<str> (interim) or remove HashMap entirely (#4)
