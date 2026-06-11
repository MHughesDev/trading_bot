# Issue #012 — Deep clone of feature payload

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Clone |
| Quick Win | No |
| Latency Impact | 1 struct copy + string clones per feature |
| Location | `crates/strategy-runtime/src/world.rs:100` |

## Problem
Feature payload is cloned into WorldState on every update, allocating strings for feature names. This occurs in the per-event update path — a frequent, hot operation proportional to feature count × tick rate.

## Root Cause
The feature payload is stored by value in `WorldState`, requiring a full clone (including the feature name String) on every update. There is no reference-counted or zero-copy mechanism for feature data.

## Note on Consolidation
This issue is fully resolved by Issue #4 (slot-array features). When #4 lands, feature names become `u16` slot indices and feature values become `f64` writes — no clone needed. This file is kept for tracking purposes.

If implementing before #4: pass `&FeatureValue` references through the update path rather than cloning. This is a partial fix that avoids string allocation while still requiring the f64 copy.

## Implementation Plan
### Step 1 — Coordinate with #4
If #4 is being implemented, this issue is resolved automatically. Verify that after #4 lands, `world.rs:100` contains no `.clone()` call on a feature payload.

### Step 2 — Interim fix (if #4 is not yet landed)
Change the update signature to accept `&FeatureValue` and store a reference or copy only the f64 value:
```rust
fn apply_feature(&mut self, name: &str, value: &FeatureValue) {
    if let Some(slot) = self.registry.get(name) {
        self.slots[*slot] = value.value;
    }
}
```
This avoids cloning the name string.

### Step 3 — Verify no clone in hot path
After either fix, use `cargo clippy -- -W clippy::clone_on_ref_ptr` and `dhat` to confirm the clone is gone.

## Acceptance Criteria
- [ ] No String clone at `world.rs:100` in the per-tick update path
- [ ] If #4 is landed: feature values are f64 slot writes with no allocation
- [ ] If #4 is not yet landed: interim fix passes &FeatureValue by reference

## Files to Change
- `crates/strategy-runtime/src/world.rs` — remove clone at line 100; coordinate with #4
