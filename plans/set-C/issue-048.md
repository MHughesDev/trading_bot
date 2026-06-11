# Issue #048 — Subscription clone in remove path

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 1 unnecessary struct copy |
| Location | `crates/ui-gateway/src/subscriptions.rs:102-108` |

## Problem
Subscription is cloned in the remove path when `remove()` would give ownership directly. The code clones the value out of the map and then removes it, paying for a struct copy that is immediately discarded.

## Root Cause
The remove code at lines 102-108 calls `.get()` to borrow the value, then clones it, then calls `.remove()`. The correct pattern is to call `.remove()` directly, which returns the owned value without cloning.

## Implementation Plan
### Step 1 — Replace get+clone+remove with remove()
Replace:
```rust
let sub = map.get(&id).unwrap().clone();
map.remove(&id);
// use sub...
```
with:
```rust
let sub = map.remove(&id).unwrap();
// use sub...
```
`HashMap::remove()` returns `Option<V>` where V is the owned value. No clone needed.

### Step 2 — Handle the case where the value is shared
If the subscription is stored as `Arc<Subscription>` (from #37), `map.remove(&id)` returns `Option<Arc<Subscription>>`. The Arc is the "owned" value — cloning it is just an atomic increment, which is fine if needed.

### Step 3 — Consolidate with #37 and other subscription issues
This is a minor fix that should be part of the same PR as the broader subscription cleanup (#25, #26, #27, #37, #40, #48).

## Acceptance Criteria
- [ ] No `.clone()` on Subscription in the remove path at `subscriptions.rs:102-108`
- [ ] `remove()` used directly to take ownership
- [ ] Subscription removal test passes

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — replace get+clone+remove with direct remove() at lines 102-108
