# Issue #026 — Subscription removal: filter+clone+iterate again

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | Full struct clone on disconnect |
| Location | `crates/ui-gateway/src/subscriptions.rs:112-128` |

## Problem
The subscription removal path collects full Subscription clones during the filter step, then iterates again to delete them. This is a two-pass algorithm with unnecessary cloning: the filter pass clones every matching struct, and the delete pass uses those clones to look up the keys to remove.

## Root Cause
The removal logic at lines 112-128 uses `.filter().cloned().collect()` to find subscriptions to remove. A more efficient pattern is to collect only the IDs (or keys) needed for removal, then do a single targeted removal pass.

## Implementation Plan
### Step 1 — Collect only IDs, not full structs
Replace:
```rust
let to_remove: Vec<Subscription> = map
    .values()
    .filter(|s| s.connection_id == conn_id)
    .cloned()
    .collect();
```
with:
```rust
let to_remove: Vec<Uuid> = map
    .iter()
    .filter(|(_, s)| s.connection_id == conn_id)
    .map(|(id, _)| *id)
    .collect();
```

### Step 2 — Single removal pass using IDs
```rust
for id in to_remove {
    map.remove(&id);
}
```
Zero clones of the Subscription struct.

### Step 3 — Apply to all removal sites
Apply this pattern to all subscription removal code in `subscriptions.rs:112-128`. This includes connection disconnect, panel close, and instrument unsubscribe paths.

### Step 4 — Consolidate with #27 (panel removal)
Panel removal at lines 132-148 has the same issue. Fix both in the same PR.

## Acceptance Criteria
- [ ] Subscription removal collects `Vec<Uuid>` of IDs, not `Vec<Subscription>` of structs
- [ ] Single removal pass using collected IDs
- [ ] Zero Subscription struct clones in the removal path
- [ ] WS disconnect test passes; all subscriptions cleaned up correctly

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — replace clone-then-remove pattern at lines 112-128 with ID-collect-then-remove
