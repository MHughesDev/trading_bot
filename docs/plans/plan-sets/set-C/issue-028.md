# Issue #028 — Subscription list: filter+clone+collect

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | Per API list call |
| Location | `crates/ui-gateway/src/subscriptions.rs:152-157` |

## Problem
The subscription list API operation clones every Subscription instead of returning references or IDs. For a UI with 100 active subscriptions, each list call allocates 100 full Subscription structs. While list calls are user-initiated and infrequent, this is a straightforward fix.

## Root Cause
At `subscriptions.rs:152-157`, the list function collects with `.cloned().collect()` rather than borrowing references.

## Implementation Plan
### Step 1 — Return Vec<&Subscription> instead of Vec<Subscription>
Change the list function signature:
```rust
// Before:
fn list_subscriptions(&self) -> Vec<Subscription>
// After:
fn list_subscriptions(&self) -> Vec<&Subscription>
```
Callers iterate over references; no allocation needed.

### Step 2 — Alternative: return Vec<Arc<Subscription>>
If Subscriptions are stored as `Arc<Subscription>` (from fix #25/#37), return `Vec<Arc<Subscription>>`. Arc clone is a cheap atomic increment.

### Step 3 — Coordinate with #25 and #37
The preferred storage type (Arc<Subscription>) should be consistent across all subscription operations. This fix should use the same type chosen in #25.

## Acceptance Criteria
- [ ] Subscription list returns references or Arc clones, not owned struct copies
- [ ] No `.cloned()` on full Subscription values at `subscriptions.rs:152-157`
- [ ] Subscription list API test passes

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — change list function to return Vec<&Subscription> or Vec<Arc<Subscription>> at lines 152-157
