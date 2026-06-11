# Issue #004 — Feature map rebuilt + every key cloned per tick

## Summary
| Field | Value |
|-------|-------|
| Severity | High |
| Phase | B |
| Pattern | Allocation |
| Quick Win | No |
| Latency Impact | 1 HashMap + one String clone per feature, per event, in most-executed function |
| Location | `crates/strategy-runtime/src/runtime.rs:65-70` |

## Problem
`process_event` copies the entire feature set into a fresh `HashMap<String, f64>` on every event — an allocation storm proportional to feature count × tick rate. At 100 features and 1000 ticks/sec per instance, this generates at least 100,000 allocations per second just for the feature map. This is the most-executed function in the system.

## Root Cause
`WorldState.features` is a `HashMap<String, FeatureValue>`. The `process_event` function rebuilds this map on every event at runtime.rs:65-70, cloning each feature name string as the HashMap key. The map is rebuilt because the code has no stable slot-indexed representation.

## Implementation Plan
### Step 1 — Build a feature name → slot ID registry at compile time
During the #3 compile step, when parsing condition expressions, collect all referenced feature names. Assign each unique name a `u16` slot index. Store the registry as `HashMap<&'static str, u16>` in the compiled instance.

### Step 2 — Change WorldState.features to `Vec<f64>`
Replace `HashMap<String, FeatureValue>` with `Vec<f64>` (pre-allocated to feature count at instance init). Use `f64::NAN` as the sentinel for "absent". Add a parallel `Vec<i64>` for `available_time` nanos if needed.

### Step 3 — Change apply_event to write by slot index
`apply_event(feature_name: &str, value: f64)` → looks up `slot_id = registry[feature_name]`, writes `slots[slot_id] = value`. No clone, no hash on hot path (registry lookup is a one-time init cost resolved at compile step).

### Step 4 — Delete the HashMap rebuild at runtime.rs:65-70
Remove the lines that build the per-tick HashMap. All downstream code must use slot indices from this point.

### Step 5 — Dependency: coordinate with #3
The `LoadFeature(u16)` opcode in the #3 bytecode evaluator reads directly from `slots[id]`. These two issues must land together or #4 must precede #3.

## Acceptance Criteria
- [ ] Zero HashMap allocations per tick in `process_event` (verified with dhat)
- [ ] `WorldState.features` is `Vec<f64>` with slot-indexed access
- [ ] Feature slot registry built once at instance init; never rebuilt per tick
- [ ] All feature writes use `slots[id] = value` pattern
- [ ] Integration test: 1000 ticks through a 50-feature strategy with zero allocations counted

## Files to Change
- `crates/strategy-runtime/src/runtime.rs` — delete per-tick HashMap rebuild at lines 65-70; update apply_event
- `crates/strategy-runtime/src/world.rs` — replace HashMap with Vec<f64> slot array
