# Issue #019 — instrument_id cloned in HashMap keys

## Summary
| Field | Value |
|-------|-------|
| Severity | Low |
| Phase | E |
| Pattern | Clone |
| Quick Win | Yes |
| Latency Impact | Per instance start/stop or grouping |
| Location | `crates/strategy-runtime/src/runtime.rs:117,140` |

## Problem
`instrument_id` String is cloned to use as a HashMap key on instance start and stop. While start/stop are not hot-path operations, this is an easy fix that also aligns with the broader interning effort.

## Root Cause
`InstanceManager` uses `HashMap<String, Vec<StrategyInstance>>` or similar, requiring owned Strings as keys. When starting or stopping an instance, the `instrument_id` from the config must be cloned to insert into the map.

## Implementation Plan
### Step 1 — Coordinate with #2 and #5
If #2 (interned IDs) and #5 (dispatch re-key) are landed, instrument IDs in the instance map are already `InstrumentId(u32)`. This issue is resolved automatically. Verify that `runtime.rs:117,140` no longer clones a String.

### Step 2 — Standalone fix (if #2/#5 not yet landed)
Change the instance map to `HashMap<Arc<str>, Vec<StrategyInstance>>`. At instance start, clone the `Arc<str>` (atomic increment) rather than the String (heap allocation). Store the `Arc<str>` on the `StrategyInstance` so that stop uses the same Arc — no second allocation.

### Step 3 — Verify fix
Confirm with clippy or dhat that no String clone occurs at `runtime.rs:117,140` after the fix.

## Acceptance Criteria
- [ ] No String clone at `runtime.rs:117,140` for instance start/stop
- [ ] instrument_id uses Arc<str> (standalone) or InstrumentId(u32) (after #2/#5)
- [ ] Instance manager tests pass

## Files to Change
- `crates/strategy-runtime/src/runtime.rs` — replace String clone at lines 117, 140 with Arc<str> or InstrumentId(u32)
