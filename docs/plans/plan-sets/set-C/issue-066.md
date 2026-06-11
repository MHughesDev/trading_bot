# Issue #066 — Venue router: async Mutex contention on lifecycle

## Summary
| Field | Value |
|-------|-------|
| Severity | Low-Medium |
| Phase | D |
| Pattern | Concurrency |
| Quick Win | No |
| Latency Impact | 1 async lock + potential queueing per collector start/stop |
| Location | `crates/venue-router/src/registry.rs:36,46,61,75` |

## Problem
`CollectorRegistry` uses tokio async Mutex for every demand/release call. With multi-strategy systems, multiple strategies may request the same instrument simultaneously, causing lock contention and queueing. Each queued lock request involves a context switch.

## Root Cause
Same root cause as #60: tokio async Mutex used where a sync Mutex or concurrent data structure would be more appropriate. The async Mutex is fair (prevents starvation) but adds overhead even in the uncontended case.

## Implementation Plan
### Step 1 — Consolidated fix with #60
Issues #60 and #66 are the same problem in the same file (`registry.rs`), at different line numbers. Fix both in a single PR.

### Step 2 — Switch to dashmap::DashMap
Replace the `Arc<tokio::sync::Mutex<HashMap<CollectorKey, u32>>>` with `Arc<DashMap<CollectorKey, u32>>`:
```rust
use dashmap::DashMap;

struct CollectorRegistry {
    counts: Arc<DashMap<CollectorKey, u32>>,
}

impl CollectorRegistry {
    fn incr(&self, key: CollectorKey) {
        *self.counts.entry(key).or_insert(0) += 1;
    }
    fn decr(&self, key: CollectorKey) {
        // use entry API for atomic decrement and remove-if-zero
    }
}
```

### Step 3 — Handle decrement and remove atomically
Use DashMap's entry API to atomically check-and-remove when count reaches 0:
```rust
self.counts.entry(key).and_modify(|c| *c -= 1).and_if(|c| *c == 0, |entry| entry.remove());
```

### Step 4 — Verify zero context switches under concurrent demand
Load test: 10 strategies simultaneously requesting the same 5 instruments. Confirm no lock queue forms (all operations complete in < 1 µs each).

## Acceptance Criteria
- [ ] No `tokio::sync::Mutex` at `registry.rs:36,46,61,75`
- [ ] DashMap used for concurrent collector count tracking
- [ ] Decrement-to-zero atomically removes the entry
- [ ] Concurrent demand from 10 strategies: no contention delay

## Files to Change
- `crates/venue-router/src/registry.rs` — replace async Mutex with DashMap at lines 36, 46, 61, 75 (consolidated with #60)
