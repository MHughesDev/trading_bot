# Issue #063 — Order intent filtering: O(n²) worst case

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Search |
| Quick Win | Yes |
| Latency Impact | 50k searches/sec at scale |
| Location | `crates/strategy-runtime/src/intents.rs:47-48` |

## Problem
Combined effect of issue #55: with 10 actions and 5 signals per strategy instance, each signal evaluation performs 50 string searches (10 × 5). With 100 strategy instances and 10 ticks/sec, this scales to 50,000 string searches per second just for intent filtering. The O(n²) complexity (actions × signals) in the signal evaluation inner loop is the key concern.

## Root Cause
Same root cause as #55: `signals` is `Vec<String>` and `.contains()` is O(n) per call. When nested inside the actions loop, the total cost is O(actions × signals) per signal evaluation.

## Implementation Plan
### Step 1 — Consolidated fix with #55
This issue is the same fix as #55 — convert `signals` to `HashSet<String>` (or `AHashSet<String>`). With O(1) set lookup, the total cost becomes O(actions × 1) = O(actions) per signal evaluation.

### Step 2 — Pre-compute signal set at intent-build time
When the strategy definition is compiled and intent configurations are built (once, at instance init), convert each action's signal list to a HashSet:
```rust
struct ActionConfig {
    on_signals: AHashSet<String>,
    // ...
}
```

### Step 3 — Verify scaling
After the fix, benchmark: 10 actions × 5 signals, 100 instances, 10 ticks/sec. Confirm the CPU time for intent filtering is O(actions), not O(actions × signals).

### Step 4 — Scale target
With AHashSet, 50,000 string searches → 10,000 O(1) lookups at ~5 ns each = ~50 µs/sec total — negligible.

## Acceptance Criteria
- [ ] Intent filtering is O(actions), not O(actions × signals)
- [ ] `AHashSet<String>` used for signal set at `intents.rs:47-48`
- [ ] Benchmark: 100 instances, 10 signals each: intent filtering < 100 µs/sec total
- [ ] All intent filter tests pass

## Files to Change
- `crates/strategy-runtime/src/intents.rs` — consolidated with #55: replace Vec<String> signals with AHashSet at lines 47-48
