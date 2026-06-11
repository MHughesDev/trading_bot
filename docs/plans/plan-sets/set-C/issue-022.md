# Issue #022 — Arc<Mutex> lock contention

## Summary
| Field | Value |
|-------|-------|
| Severity | Low-Medium |
| Phase | D |
| Pattern | Concurrency |
| Quick Win | No |
| Latency Impact | Up to 100 µs per cycle |
| Location | `crates/storage/src/writer.rs:42` |

## Problem
`Arc<Mutex>` is used for shared state accessed from multiple tokio tasks. Under load, contention on this mutex causes context switches that can stall operations for up to 100 µs — significant for a system targeting sub-millisecond decision latency. The storage writer is not on the hot path, but contention can cascade.

## Root Cause
`Arc<Mutex<HashMap>>` is the default Rust shared-state pattern and was used without analyzing actual contention. Tokio's async Mutex adds context-switch overhead when the lock is contended because awaiting a Mutex yields the task to the scheduler.

## Implementation Plan
### Step 1 — Audit all Mutex uses in storage/demand-manager/venue-router
List every `Arc<Mutex<...>>` and `Arc<RwLock<...>>` in:
- `crates/storage/src/writer.rs`
- `crates/demand-manager/src/registry.rs`
- `crates/venue-router/src/registry.rs`

Classify each as: (a) high-contention shared map, (b) low-contention sync, (c) could be actor-model.

### Step 2 — Replace high-contention maps with dashmap
For `HashMap` or `HashSet` values protected by Mutex where multiple concurrent readers/writers exist, replace with `dashmap::DashMap`. DashMap is a concurrent hashmap that shards internally to reduce contention.

### Step 3 — Replace low-contention sync with parking_lot::Mutex
For state accessed infrequently or under low concurrency, replace `std::sync::Mutex` with `parking_lot::Mutex`. parking_lot is faster than std Mutex for uncontended and lightly-contended cases.

### Step 4 — Convert actor-model candidates to single-threaded tasks
For state that is accessed from many places but could be owned by a single task, refactor to use message-passing (mpsc channel). The task owns the state exclusively; no locking needed.

## Acceptance Criteria
- [ ] All high-contention maps use dashmap instead of Arc<Mutex<HashMap>>
- [ ] Low-contention sync uses parking_lot::Mutex
- [ ] No tokio async Mutex used where sync Mutex suffices
- [ ] Benchmark shows reduction in lock contention under 100-task load

## Files to Change
- `crates/storage/src/writer.rs` — audit and replace Mutex at line 42
- `crates/demand-manager/src/registry.rs` — replace Mutex where appropriate
- `crates/venue-router/src/registry.rs` — replace Mutex where appropriate
