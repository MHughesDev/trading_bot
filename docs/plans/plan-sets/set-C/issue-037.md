# Issue #037 — Subscription fully cloned at insertion

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | 2–3 struct copies |
| Location | `crates/ui-gateway/src/subscriptions.rs:84-96` |

## Problem
The Subscription struct is cloned 2-3 times during insertion into multiple data structures. The subscription registry stores subscriptions in more than one data structure simultaneously (e.g., keyed by connection_id AND by subscription_id), requiring separate owned copies.

## Root Cause
Multiple `HashMap` or `Vec` entries at the subscription registry hold independent owned copies of the same `Subscription`. Since Rust requires ownership, each insertion clones the struct.

## Implementation Plan
### Step 1 — Wrap Subscription in Arc at the point of creation
```rust
let sub = Arc::new(Subscription { ... });
self.by_id.insert(sub.id, Arc::clone(&sub));
self.by_connection.entry(sub.connection_id).or_default().push(Arc::clone(&sub));
```
All data structures hold `Arc<Subscription>`. Three logical "copies" = one allocation + two atomic increments.

### Step 2 — Update all data structure types
```rust
by_id: HashMap<Uuid, Arc<Subscription>>,
by_connection: HashMap<Uuid, Vec<Arc<Subscription>>>,
```

### Step 3 — Update all access sites
Code that currently dereferences a Subscription from the map should now dereference through `Arc`. In most cases this is transparent — `(*sub).field` or `sub.field` (auto-deref).

### Step 4 — Consolidate with #25, #26, #27, #28, #48
All subscription-related clone issues (#25, #26, #27, #28, #37, #48) should be fixed together by switching to `Arc<Subscription>` storage. One PR, one code review.

## Acceptance Criteria
- [ ] Subscription allocated once at creation; stored as Arc<Subscription> everywhere
- [ ] Zero Subscription struct copies at lines 84-96
- [ ] Subscribe, unsubscribe, list, disconnect all use Arc<Subscription>
- [ ] WS subscription integration test passes

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — wrap Subscription in Arc; update all storage types at lines 84-96 and throughout file
