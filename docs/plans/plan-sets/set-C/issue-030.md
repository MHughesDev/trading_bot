# Issue #030 — Demand registry: lock contention with nested unwraps

## Summary
| Field | Value |
|-------|-------|
| Severity | Low-Medium |
| Phase | D |
| Pattern | Concurrency |
| Quick Win | No |
| Latency Impact | 2 lock acquisitions per operation |
| Location | `crates/demand-manager/src/registry.rs:55,66,77,102` |

## Problem
Multiple `.lock().unwrap()` calls per operation in the demand registry. Nested locks risk contention (two separate critical sections that could be atomically combined) and panic on poisoning (any thread that panics while holding the lock poisons it, causing all subsequent `.unwrap()` calls to panic).

## Root Cause
The demand registry uses standard `Mutex` with multiple operations requiring separate lock acquisitions. Each `.lock().unwrap()` at lines 55, 66, 77, and 102 acquires and releases the lock independently, leaving the registry in intermediate states between acquisitions.

## Implementation Plan
### Step 1 — Replace with dashmap::DashMap
For the primary demand tracking map, replace `Mutex<HashMap<...>>` with `dashmap::DashMap`. DashMap provides concurrent access without explicit locking:
```rust
use dashmap::DashMap;
struct DemandRegistry {
    demands: DashMap<InstrumentId, u32>,
}
```
Removes all `.lock().unwrap()` calls at lines 55, 66, 77, 102.

### Step 2 — Alternative: consolidate into a single-threaded actor task
Move the demand registry into a tokio task that owns the state exclusively. External callers communicate via mpsc channels. No locking needed — the task processes one message at a time. Better for complex state transitions.

### Step 3 — Eliminate panic risk
Replace all `.unwrap()` on lock results with `.expect("demand registry lock poisoned")` or use `parking_lot::Mutex` which never poisons (panics at call site instead).

### Step 4 — Consolidate with #22 (Arc<Mutex> lock contention)
This is a specific instance of the same issue as #22. Coordinate the fix to use a consistent approach across all registries.

## Acceptance Criteria
- [ ] Zero `.lock().unwrap()` calls at `registry.rs:55,66,77,102`
- [ ] No mutex poisoning risk on the demand registry
- [ ] DashMap or actor-task pattern used consistently
- [ ] Demand registry under concurrent load: no deadlock or panic

## Files to Change
- `crates/demand-manager/src/registry.rs` — replace Mutex+unwrap pattern at lines 55, 66, 77, 102 with dashmap or actor-task
