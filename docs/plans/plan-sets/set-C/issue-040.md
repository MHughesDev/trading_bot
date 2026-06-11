# Issue #040 — Lock/unwrap chains creating contention

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | D |
| Pattern | Concurrency |
| Quick Win | No |
| Latency Impact | 3–4 lock cycles per subscription operation |
| Location | `crates/ui-gateway/src/subscriptions.rs:96,102,114,121` |

## Problem
Multiple lock acquisitions per subscribe/unsubscribe operation in the UI gateway subscriptions. Each `.lock().unwrap()` at lines 96, 102, 114, and 121 acquires and releases the lock independently, creating 3-4 lock cycles per operation and leaving the subscription state inconsistent between acquisitions.

## Root Cause
The subscription registry is protected by a `Mutex` or `RwLock`, and the subscribe/unsubscribe logic requires multiple operations on multiple data structures that are each independently locked. This leads to contention under high subscription churn.

## Implementation Plan
### Step 1 — Replace subscription HashMap with dashmap::DashMap
For the primary subscription lookup map, use `dashmap::DashMap`:
```rust
subscriptions: DashMap<Uuid, Arc<Subscription>>,
by_connection: DashMap<Uuid, Vec<Arc<Subscription>>>,
```
All concurrent reads and writes are handled by DashMap's internal sharding; no explicit lock acquisitions.

### Step 2 — Eliminate nested locking patterns
Redesign operations that need to update two maps atomically to avoid the inconsistency window. Use DashMap's entry API for atomic check-and-insert.

### Step 3 — Alternative: actor task for subscription management
Move subscription state into a dedicated tokio task. External code sends `Subscribe`, `Unsubscribe`, `ListSubscriptions` messages via mpsc channels. The task processes one message at a time — no locks needed. Subscription changes are atomic from the task's perspective.

### Step 4 — Consolidate with #37 and Arc<Subscription>
This fix should use the `Arc<Subscription>` storage from #37 — consistent approach across all subscription issues.

## Acceptance Criteria
- [ ] Zero explicit `.lock().unwrap()` calls at `subscriptions.rs:96,102,114,121`
- [ ] No risk of inconsistent subscription state between lock acquisitions
- [ ] DashMap or actor task pattern used
- [ ] Subscription operations safe under concurrent WS connections

## Files to Change
- `crates/ui-gateway/src/subscriptions.rs` — replace lock/unwrap chain at lines 96, 102, 114, 121 with DashMap or actor-task
