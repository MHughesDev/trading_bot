# Issue #005 — Dispatch scans all instances linearly with string compares + clones event per match

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Search |
| Quick Win | No |
| Latency Impact | O(total instances) per event instead of O(instances on this instrument); 1 deep WorldEvent clone per match |
| Location | `crates/strategy-runtime/src/runtime.rs:166-181` |

## Problem
Every event iterates the entire instance map comparing owned String instrument IDs, then deep-clones the event per matching instance. With 50 instruments × 3 strategies each = 150 instances, every tick for one instrument scans all 150 instances and performs string compares. The clone adds heap allocation on top of the O(n) scan.

## Root Cause
`InstanceManager.instances` is keyed by something other than `InstrumentId`, so the dispatcher cannot do a direct O(1) bucket lookup. The string instrument ID in `WorldEvent` must be compared against each instance's instrument string. The subsequent `event.clone()` at runtime.rs:174 copies the entire event for each matching instance.

## Implementation Plan
### Step 1 — Re-key InstanceManager on InstrumentId (depends on #2)
Change `InstanceManager.instances` from its current map type to `HashMap<InstrumentId, Vec<StrategyInstance>>` using the interned `u32` from issue #2. This requires #2 to land first (or be coordinated).

### Step 2 — Update dispatch signature
Change `dispatch` to accept `instrument: InstrumentId` and `event: &WorldEvent` (by reference). The single O(1) HashMap bucket lookup replaces the linear scan.

### Step 3 — Pass event by reference to instance evaluation
Update `StrategyInstance::process_event` to accept `&WorldEvent` instead of `WorldEvent`. Eliminate the `event.clone()` at runtime.rs:174.

### Step 4 — Update WorldEvent instrument_id fields
Change `WorldEvent`'s String `instrument_id` fields to `InstrumentId(u32)`. Update all construction sites.

### Step 5 — Verify O(1) dispatch
Add a benchmark: 200 instances across 50 instruments, dispatch 1000 events to one instrument. Confirm time is constant with respect to total instance count.

## Acceptance Criteria
- [ ] Dispatch cost is O(instances on this instrument), not O(total instances)
- [ ] Zero `event.clone()` calls in the dispatch path
- [ ] `InstanceManager.instances` keyed by `InstrumentId(u32)`
- [ ] WorldEvent instrument_id is `InstrumentId(u32)`, not String
- [ ] Benchmark confirms O(1) dispatch with 200 total instances

## Files to Change
- `crates/strategy-runtime/src/runtime.rs` — re-key InstanceManager, update dispatch, remove event clone at line 174
