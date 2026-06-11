# Issue #055 — Intent filtering: signals.contains() is O(n)

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Search |
| Quick Win | Yes |
| Latency Impact | 15 searches per signal eval × scale |
| Location | `crates/strategy-runtime/src/intents.rs:47` |

## Problem
`signals.contains(&a.on_signal)` searches a `Vec<String>` for every action check. With 5 actions × 3 signals per strategy instance, each signal evaluation performs 15 string comparisons. This scales poorly with the number of actions and signals configured per strategy.

## Root Cause
`signals` is stored as `Vec<String>`. The `.contains()` method on Vec is O(n), requiring a linear scan. For a set-membership check, `HashSet` provides O(1) average-case lookup.

## Implementation Plan
### Step 1 — Convert signals from Vec<String> to HashSet<String> at intent-build time
When building the intent configuration (not per tick), convert the signals list:
```rust
let signal_set: HashSet<String> = signals.into_iter().collect();
```
Store `signal_set` on the intent or action configuration.

### Step 2 — Use signal_set.contains() instead of signals.contains()
Replace:
```rust
signals.contains(&a.on_signal)
```
with:
```rust
signal_set.contains(&a.on_signal)
```
O(1) average-case lookup, no linear scan.

### Step 3 — Consider AHashSet for faster hashing
If the signal name set is small (< 20 items), `ahash::AHashSet` is faster than `std::HashSet` due to a faster non-cryptographic hash function.

### Step 4 — Consolidate with #63
Issue #63 describes the combined O(n²) effect of this issue. Fix both by implementing the HashSet approach once.

## Acceptance Criteria
- [ ] `signals` stored as `HashSet<String>` on the intent configuration
- [ ] `signal_set.contains()` used at `intents.rs:47` instead of Vec contains
- [ ] Signal membership check is O(1) per action evaluation
- [ ] Intent filter tests pass with correct signal matching
- [ ] Benchmark: 10 actions × 5 signals: constant time regardless of signal count

## Files to Change
- `crates/strategy-runtime/src/intents.rs` — convert signals to HashSet at intent-build time; use set lookup at line 47
