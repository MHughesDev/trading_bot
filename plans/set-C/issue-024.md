# Issue #024 — Expressions parsed per evaluation in universe

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | B |
| Pattern | Parsing |
| Quick Win | No |
| Latency Impact | Per-entry parsing × universe size |
| Location | `crates/strategy-runtime/src/nodes/filter.rs:12-18` |

## Problem
The universe filter node re-parses expression strings on every evaluation. With a universe of 50 instruments, each tick causes 50 full parse cycles for a single filter expression — the same string every time. This is the same root issue as #3 but located in the filter/rank pipeline rather than the condition evaluator.

## Root Cause
The filter node at `filter.rs:12-18` calls the string-based expression evaluator directly, without a compiled representation. Expressions are frozen at strategy load time but are re-parsed per evaluation.

## Implementation Plan
### Step 1 — Coordinate with #3 (bytecode compiler)
The same bytecode compiler and evaluator from #3 applies here. At filter node init, compile the filter expression to bytecode (`Vec<Op>`). Store on the filter node.

### Step 2 — Change filter node evaluation to use bytecode
At evaluation time, call `evaluate_program(&self.compiled_expr, &entry.slots)` for each universe entry. No parser call, no string handling.

### Step 3 — Apply to rank node expressions
If the rank node also evaluates string expressions, apply the same compile-once pattern.

### Step 4 — Verify zero allocations in filter/rank hot path
Run a benchmark with a 50-entry universe and 5-stage filter/rank pipeline. Confirm no allocations from parsing.

## Acceptance Criteria
- [ ] Filter node compiles expression at init time; never calls parser per tick
- [ ] Rank node (if applicable) also compiles at init time
- [ ] Zero parser allocations per tick in filter/rank pipeline
- [ ] All existing filter/rank tests pass

## Files to Change
- `crates/strategy-runtime/src/nodes/filter.rs` — add compile-at-init step; replace per-evaluation parse with bytecode execution at lines 12-18
