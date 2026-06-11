# Issue #043 — RateBudget: lock/unwrap on every check

## Summary
| Field | Value |
|-------|-------|
| Severity | Very Low |
| Phase | E |
| Pattern | Concurrency |
| Quick Win | Yes |
| Latency Impact | Mutex lock per rate-limit check |
| Location | `crates/demand-manager/src/rate_budget.rs:76,92,100` |

## Problem
`RateBudget` uses `Mutex<u32>` for the current count; this Mutex is locked on every rate-limit check. Rate-limit checks may occur on every WS frame or tick, making this a frequently-acquired lock for a simple counter operation.

## Root Cause
A `Mutex<u32>` is used where an `AtomicU32` would suffice. Mutex incurs kernel-level synchronization overhead (futex system call under contention) for what is logically a simple atomic increment and compare.

## Implementation Plan
### Step 1 — Replace Mutex<u32> with AtomicU32
```rust
use std::sync::atomic::{AtomicU32, Ordering};

struct RateBudget {
    current: AtomicU32,
    max: u32,
    window_ns: u64,
}
```

### Step 2 — Use fetch_add with Relaxed ordering for count increment
```rust
fn check_and_increment(&self) -> bool {
    let current = self.current.fetch_add(1, Ordering::Relaxed);
    current < self.max
}
```
`Ordering::Relaxed` is sufficient if the count is only used for rate limiting (not for synchronization with other data).

### Step 3 — Implement window reset with AtomicU64 timestamp
If the budget resets per time window, use an `AtomicU64` for the window start timestamp. Compare-exchange to reset the counter atomically when the window expires.

### Step 4 — Remove .lock().unwrap() at lines 76, 92, 100
Delete all Mutex acquire calls. Replace with atomic operations.

## Acceptance Criteria
- [ ] No `Mutex<u32>` at `rate_budget.rs:76,92,100`
- [ ] `AtomicU32` used for the counter
- [ ] Rate limit checks are lock-free (no kernel syscall)
- [ ] Rate budget tests pass: correct limiting at 0, at max, and at overflow

## Files to Change
- `crates/demand-manager/src/rate_budget.rs` — replace Mutex<u32> with AtomicU32; update operations at lines 76, 92, 100
