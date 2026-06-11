# Issue #045 — Throttle: atomic-like lock per WS frame

## Summary
| Field | Value |
|-------|-------|
| Severity | Low-Medium |
| Phase | E |
| Pattern | Concurrency |
| Quick Win | Yes |
| Latency Impact | Mutex lock per WS frame (high volume) |
| Location | `crates/ui-gateway/src/throttle.rs:69,81,98,104` |

## Problem
`Mutex<u32>` is used for the throttle counter and locked on every outgoing WS frame. With a UI receiving 100 frames/sec and the throttle lock acquired and released per frame, this is 100 Mutex acquisitions per second per connection — unnecessary kernel-level synchronization for a simple counter.

## Root Cause
Same root cause as #43 (RateBudget): a `Mutex<u32>` is used where `AtomicU32` would suffice. The throttle counter is a simple numeric value with no complex invariants requiring mutex-level protection.

## Implementation Plan
### Step 1 — Replace Mutex<u32> with AtomicU32
```rust
use std::sync::atomic::{AtomicU32, Ordering};

struct WsThrottle {
    frame_count: AtomicU32,
    max_fps: u32,
}
```

### Step 2 — Use fetch_add with Relaxed ordering
```rust
fn should_send(&self) -> bool {
    let count = self.frame_count.fetch_add(1, Ordering::Relaxed);
    count < self.max_fps
}
```

### Step 3 — Implement window reset with compare_exchange
For time-windowed throttling, use `AtomicU64` for the window timestamp and `compare_exchange` to atomically reset the counter when the window expires. No Mutex needed.

### Step 4 — Remove .lock().unwrap() at lines 69, 81, 98, 104
Delete all Mutex lock calls. Replace with atomic fetch_add / compare_exchange.

### Step 5 — Consolidate with #43
Same fix pattern as #43. Can be done in the same PR.

## Acceptance Criteria
- [ ] No `Mutex<u32>` at `throttle.rs:69,81,98,104`
- [ ] `AtomicU32` used for frame count
- [ ] WS throttle check is lock-free (no kernel syscall under non-contention)
- [ ] Throttle tests pass: frames correctly limited at max_fps

## Files to Change
- `crates/ui-gateway/src/throttle.rs` — replace Mutex<u32> with AtomicU32; update operations at lines 69, 81, 98, 104
