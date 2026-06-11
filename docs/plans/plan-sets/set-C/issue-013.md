# Issue #013 — Universe cloned across pipeline

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Clone |
| Quick Win | No |
| Latency Impact | O(n) copy × stage count |
| Location | `crates/strategy-runtime/src/nodes/mod.rs:46,53,59,65,71` |

## Problem
The Universe is cloned at each pipeline stage (filter, rank, limit) instead of being threaded by reference. With a universe of 50 instruments across 5 pipeline stages, each tick creates and discards 5 full universe copies. This multiplies allocation cost proportionally to pipeline depth.

## Root Cause
Pipeline stage functions take `Universe` by value rather than by reference or `Arc`. Each stage clones the input, applies its transformation, and passes the clone to the next stage. The original design prioritized simplicity over performance.

## Implementation Plan
### Step 1 — Audit all pipeline stage signatures
Review `filter`, `rank`, `limit`, and any other nodes in `crates/strategy-runtime/src/nodes/mod.rs`. Identify all sites at lines 46, 53, 59, 65, 71 where Universe is cloned.

### Step 2 — Wrap Universe in Arc
Change the pipeline to thread `Arc<Universe>` (or `Arc<Vec<UniverseEntry>>`) through stages. Stages that do not modify the universe forward the same Arc. Only stages that modify the universe create a new allocation (clone-on-write semantic using `Arc::make_mut`).

### Step 3 — Alternative: thread by reference with slices
If the pipeline is linear (no branching), pass `&Universe` through all stages. Return results as indices into the original universe rather than copying entries. Only materialize a new Vec at the terminal stage.

### Step 4 — Benchmark both approaches
Measure allocation count per tick with a 50-entry universe and 5-stage pipeline. Choose the approach with zero allocations for non-modifying stages.

### Step 5 — Update node function signatures
Change function signatures in `nodes/mod.rs` and related node files to accept references or Arc. Update all call sites.

## Acceptance Criteria
- [ ] Non-modifying pipeline stages forward Universe without allocation
- [ ] Total allocations per pipeline pass proportional to number of modifying stages only (ideally 0 or 1)
- [ ] All node functions updated to accept references or Arc<Universe>
- [ ] Pipeline integration test with 50-entry universe passes

## Files to Change
- `crates/strategy-runtime/src/nodes/mod.rs` — change pipeline stage signatures; remove clone at lines 46, 53, 59, 65, 71
- Related node files (filter.rs, rank.rs, limit.rs) — update to accept &Universe or Arc<Universe>
