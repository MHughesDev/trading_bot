# Issue #060 — CollectorRegistry: async Mutex overhead

## Summary
| Field | Value |
|-------|-------|
| Severity | Low-Medium |
| Phase | D |
| Pattern | Concurrency |
| Quick Win | No |
| Latency Impact | 1 async lock acquisition (~1-3 µs + context switch) per demand change |
| Location | `crates/venue-router/src/registry.rs:18-19` |

## Problem
`Arc<Mutex<HashMap>>` with `.lock().await` (tokio async Mutex) is used on every demand increment/decrement. The tokio Mutex is fair and asynchronous — it yields the task when contended, causing a context switch. Even uncontended, it is heavier than a sync Mutex for operations that are brief.

## Root Cause
`tokio::sync::Mutex` was used to allow locking across .await points (required if the critical section contains async code). If the HashMap operations inside the lock are synchronous, a sync Mutex (`parking_lot::Mutex` or `std::sync::Mutex`) is more appropriate.

## Implementation Plan
### Step 1 — Audit whether the critical section contains async code
Read `registry.rs:18-19` and the surrounding lock scope. If the code inside the lock is purely synchronous HashMap operations (insert, remove, get), switch to sync Mutex.

### Step 2 — Replace tokio::sync::Mutex with dashmap::DashMap (preferred)
For a concurrent HashMap, `dashmap::DashMap` provides concurrent access without any explicit locking. Replace the `Arc<Mutex<HashMap>>` with `Arc<DashMap>`:
```rust
registry: Arc<DashMap<CollectorKey, u32>>,
```
All `incr` and `decr` operations use DashMap's concurrent entry API; no locks visible in code.

### Step 3 — Alternative: sync Mutex if access patterns are brief
If DashMap is not appropriate, replace `tokio::sync::Mutex` with `parking_lot::Mutex` (sync). Brief sync Mutex acquisitions are faster than async Mutex for non-async critical sections.

### Step 4 — Consolidate with #66
Issue #66 describes the same problem in the same file. Fix both in a single PR.

## Acceptance Criteria
- [ ] No `tokio::sync::Mutex` for HashMap operations at `registry.rs:18-19`
- [ ] DashMap or sync Mutex used instead
- [ ] Demand increment/decrement: no context switch on uncontended path
- [ ] Collector registry unit tests pass

## Files to Change
- `crates/venue-router/src/registry.rs` — replace async Mutex with DashMap or sync Mutex at lines 18-19
