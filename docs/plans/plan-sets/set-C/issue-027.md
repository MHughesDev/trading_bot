# Issue #027 — Panel removal: two-pass with clones

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | Identical to #026 |
| Location | `crates/ui-gateway/src/subscriptions.rs:132-148` |

## Problem
Panel removal uses the same two-pass clone-then-delete pattern as subscription removal (#26). Matching panels are cloned into a Vec during the filter pass, then iterated again to delete them. Same inefficiency, same fix.

## Root Cause
Same root cause as #26: `.filter().cloned().collect()` used to find panels to remove, producing unnecessary struct copies. The correct pattern is to collect only the panel IDs needed for deletion.

## Implementation Plan
### Step 1 — Apply the same fix as #26
Replace:
```rust
let to_remove: Vec<Panel> = panels
    .values()
    .filter(|p| p.connection_id == conn_id)
    .cloned()
    .collect();
```
with:
```rust
let to_remove: Vec<Uuid> = panels
    .iter()
    .filter(|(_, p)| p.connection_id == conn_id)
    .map(|(id, _)| *id)
    .collect();
for id in to_remove {
    panels.remove(&id);
}
```

### Step 2 — Consolidate with #26
Fix both #26 and #27 in the same PR and the same code review pass through `subscriptions.rs`. They are in adjacent code sections.

## Acceptance Criteria
- [ ] Panel removal collects `Vec<Uuid>` of IDs, not `Vec<Panel>` of structs
- [ ] Zero Panel struct clones in the removal path
- [ ] Panel close / connection disconnect tests pass

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — replace clone-then-remove at lines 132-148 with ID-collect-then-remove
