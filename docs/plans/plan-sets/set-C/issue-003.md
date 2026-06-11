# Issue #003 — Interpreter re-parses expression strings every event

## Summary
| Field | Value |
|-------|-------|
| Severity | High |
| Phase | B |
| Pattern | Parsing |
| Quick Win | No |
| Latency Impact | Full tokenize + parse per condition per tick (µs + allocs) |
| Location | `crates/strategy-runtime/src/interpreter.rs:58-98` |

## Problem
`evaluate_condition("feature('ema_7') > feature('ema_21')", …)` re-lexes and re-parses the same frozen string on every event, for every condition node, for every strategy instance. The expression strings are immutable after strategy load — there is no reason to parse them more than once. This is the dominant CPU cost in the strategy evaluation hot loop.

## Root Cause
The interpreter takes condition expressions as raw strings and runs a full recursive-descent tokenize+parse pipeline on every call to `evaluate_condition`. There is no compiled representation caching; the parser was written for simplicity rather than throughput.

## Implementation Plan
### Step 1 — Define a bytecode instruction set
Create `crates/strategy-runtime/src/bytecode.rs` with the `Op` enum:
```rust
pub enum Op {
    LoadFeature(u16),   // push slots[id]
    LoadBar(u8),        // push bar field
    Const(f64),         // push literal
    Add, Sub, Mul, Div, // arithmetic
    Gt, Lt, Ge, Le, Eq, Ne, // comparison (leave bool as f64 1.0/0.0)
    Neg,                // unary negate
}
pub type Program = Vec<Op>;
```

### Step 2 — Add a compile step to StrategyInstance init
At instance initialization (not per-tick), call a new `compile(expr: &str) -> Program` function that runs the existing recursive-descent parser and emits postfix bytecode. Store compiled programs in a `HashMap<NodeId, Program>` on the instance.

### Step 3 — Implement the bytecode evaluator
Add `evaluate_program(program: &[Op], slots: &[f64]) -> f64` in `bytecode.rs`. Use a fixed-size stack (`[f64; 32]` on the stack). No heap allocation.

### Step 4 — Wire bytecode into `evaluate_signals`
In `crates/strategy-runtime/src/runtime.rs`, replace the call to `evaluate_condition(expr_string, …)` with `evaluate_program(instance.programs[node_id], &world_state.slots)`. The string-based path becomes compile-only, never called per tick.

### Step 5 — Verify zero allocations with dhat
Run the strategy evaluation hot loop under `dhat-rs` or `cargo bench` with allocation counting. Confirm zero heap allocations in `process_event` evaluation.

## Acceptance Criteria
- [ ] Zero heap allocations during `process_event` evaluation (verified with `dhat` or alloc counter)
- [ ] All condition expressions compiled at instance init time, not per-tick
- [ ] Bytecode evaluator uses stack-allocated fixed array (no Vec growth per evaluation)
- [ ] Existing parser remains as compile front-end; all tests pass

## Files to Change
- `crates/strategy-runtime/src/interpreter.rs` — demote to compiler front-end only; remove per-tick parse calls
- `crates/strategy-runtime/src/bytecode.rs` — new file: Op enum, Program type, evaluate_program
- `crates/strategy-runtime/src/runtime.rs` — wire evaluate_program into evaluate_signals
