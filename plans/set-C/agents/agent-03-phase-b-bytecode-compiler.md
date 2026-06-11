# Agent Query — Compile Strategy Expressions to Postfix Bytecode
## Covers Issues: #3, #24
## Phase: B
## Estimated Effort: 1–2 weeks
## Prerequisites: #1 (in-process ring pipeline must exist); #4 can be done simultaneously since bytecode operands use slot IDs

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

The strategy runtime evaluates boolean conditions like `feature('ema_7') > feature('ema_21')` by running a full tokenize-and-parse pipeline on every market event, for every condition node, for every strategy instance. These expression strings are immutable after the strategy definition is loaded — they never change at runtime. Running the parser per-tick is pure wasted CPU. The same re-parsing pattern also appears in the universe filter node. Together these dominate the CPU cost in the strategy hot loop. The fix is to compile every condition expression to a postfix bytecode program once at instance initialization and execute the bytecode on each tick — zero parsing, zero allocation.

## Codebase Context

- `crates/strategy-runtime/src/interpreter.rs` — around lines 58–98, `evaluate_condition(expr: &str, ...)` re-lexes and re-parses the expression string on every call. This is the hot path.
- `crates/strategy-runtime/src/runtime.rs` — calls `evaluate_condition(...)` in the event-processing loop.
- `crates/strategy-runtime/src/nodes/filter.rs` — around lines 12–18, the universe filter node applies the same re-parse pattern to filter expressions.
- No `bytecode.rs` file currently exists in `crates/strategy-runtime/src/`.

The problematic hot-loop pattern in `runtime.rs`:
```rust
// Called on every market event, for every condition in every strategy instance
for condition in &instance.conditions {
    let result = evaluate_condition(&condition.expr, &world_state)?;
    // ...
}
```

And in `interpreter.rs`:
```rust
pub fn evaluate_condition(expr: &str, world: &WorldState) -> Result<bool> {
    let tokens = tokenize(expr)?;   // ← allocates Vec<Token> every call
    let ast = parse(tokens)?;       // ← allocates AST nodes every call
    eval_expr(&ast, world)
}
```

## Task

### Fix #3 — Compile conditions at instance init (main interpreter)

**Problem:** `evaluate_condition(expr: &str, ...)` in `crates/strategy-runtime/src/interpreter.rs` (around lines 58–98) re-lexes and re-parses the expression on every call. This is invoked once per condition per strategy instance per market event.

**Solution:** At `StrategyInstance::new` (initialization time), compile every condition expression to a `Program` (a `Vec<Op>` of postfix bytecode). At evaluation time, execute the `Program` against the slot array — zero parsing, zero heap allocation.

**Implementation steps:**

1. Create `crates/strategy-runtime/src/bytecode.rs` with the following content:

```rust
/// Postfix bytecode instruction set for strategy condition evaluation.
/// Programs are compiled once at instance init and executed per tick.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    LoadFeature(u16),   // push feature_slots[id] onto stack
    LoadBarOpen,        // push current bar open price
    LoadBarHigh,        // push current bar high price
    LoadBarLow,         // push current bar low price
    LoadBarClose,       // push current bar close price
    LoadBarVolume,      // push current bar volume
    Const(f64),         // push literal constant value
    Add,
    Sub,
    Mul,
    Div,
    Neg,                // negate top of stack
    Gt,                 // pop a, b; push (b > a) as 1.0 or 0.0
    Lt,
    Ge,
    Le,
    Eq,
    Ne,
    And,                // logical and (0.0 == false)
    Or,                 // logical or
    Not,                // logical not
}

pub type Program = Vec<Op>;

/// Execute a compiled program against a feature slot array.
/// Uses a fixed 32-element stack — zero heap allocation per call.
/// Returns the top-of-stack value (non-zero == true for boolean conditions).
#[inline(always)]
pub fn run(program: &[Op], slots: &[f64], bar: &crate::world::BarSnapshot) -> f64 {
    let mut stack = [0f64; 32];
    let mut sp = 0usize;
    for op in program {
        match op {
            Op::LoadFeature(id) => {
                stack[sp] = slots[*id as usize];
                sp += 1;
            }
            Op::LoadBarOpen   => { stack[sp] = bar.open;   sp += 1; }
            Op::LoadBarHigh   => { stack[sp] = bar.high;   sp += 1; }
            Op::LoadBarLow    => { stack[sp] = bar.low;    sp += 1; }
            Op::LoadBarClose  => { stack[sp] = bar.close;  sp += 1; }
            Op::LoadBarVolume => { stack[sp] = bar.volume; sp += 1; }
            Op::Const(v) => { stack[sp] = *v; sp += 1; }
            Op::Add => { sp -= 1; stack[sp - 1] += stack[sp]; }
            Op::Sub => { sp -= 1; stack[sp - 1] -= stack[sp]; }
            Op::Mul => { sp -= 1; stack[sp - 1] *= stack[sp]; }
            Op::Div => { sp -= 1; stack[sp - 1] /= stack[sp]; }
            Op::Neg => { stack[sp - 1] = -stack[sp - 1]; }
            Op::Gt  => { sp -= 1; stack[sp-1] = (stack[sp-1] > stack[sp]) as u8 as f64; }
            Op::Lt  => { sp -= 1; stack[sp-1] = (stack[sp-1] < stack[sp]) as u8 as f64; }
            Op::Ge  => { sp -= 1; stack[sp-1] = (stack[sp-1] >= stack[sp]) as u8 as f64; }
            Op::Le  => { sp -= 1; stack[sp-1] = (stack[sp-1] <= stack[sp]) as u8 as f64; }
            Op::Eq  => { sp -= 1; stack[sp-1] = (stack[sp-1] == stack[sp]) as u8 as f64; }
            Op::Ne  => { sp -= 1; stack[sp-1] = (stack[sp-1] != stack[sp]) as u8 as f64; }
            Op::And => { sp -= 1; stack[sp-1] = ((stack[sp-1] != 0.0) && (stack[sp] != 0.0)) as u8 as f64; }
            Op::Or  => { sp -= 1; stack[sp-1] = ((stack[sp-1] != 0.0) || (stack[sp] != 0.0)) as u8 as f64; }
            Op::Not => { stack[sp-1] = (stack[sp-1] == 0.0) as u8 as f64; }
        }
    }
    if sp > 0 { stack[0] } else { 0.0 }
}
```

2. In `interpreter.rs`, add a `compile` function that takes an expression string and a slot-resolver closure, then walks the existing recursive-descent parser emitting `Op` values instead of evaluating them:

```rust
pub fn compile(
    expr: &str,
    slot_resolver: &dyn Fn(&str) -> Option<u16>,
) -> Result<Program, CompileError> {
    let tokens = tokenize(expr)?;
    let mut program = Vec::new();
    compile_expr(&tokens, &mut 0, &mut program, slot_resolver)?;
    Ok(program)
}
```

The existing `eval_expr` recursive-descent structure is transformed into `compile_expr` — same grammar rules, same operator precedence, but instead of computing values it emits `Op` instructions in postfix order.

3. In `StrategyInstance::new` (or wherever instances are constructed), call `compile()` for every condition node expression. The `slot_resolver` closure queries the `FeatureRegistry` (see agent-04) to get the `u16` slot ID for each feature name. Store the compiled programs:

```rust
pub struct StrategyInstance {
    // ... existing fields ...
    pub compiled_conditions: HashMap<NodeId, Program>,
}
```

4. In `crates/strategy-runtime/src/runtime.rs`, replace the call to `evaluate_condition(expr, &world_state)` in the hot loop with:

```rust
let result = bytecode::run(
    &instance.compiled_conditions[&node_id],
    &world_state.feature_slots,
    &world_state.current_bar,
) != 0.0;
```

5. Delete the string-based `evaluate_condition` function from `interpreter.rs` (or rename it `compile_and_evaluate` and restrict it to the compile-time path only). The hot-loop path must not call `tokenize` or `parse`.

### Fix #24 — Compile universe filter expressions at node init

**Problem:** The universe filter node in `crates/strategy-runtime/src/nodes/filter.rs` (around lines 12–18) re-parses filter expressions on every `filter()` call. The same re-parse issue as #3, just in a different node type.

**Solution:** Apply the same bytecode compilation approach. At node creation time, call `bytecode::compile()` for each filter expression. At filter time, call `bytecode::run()`.

**Implementation steps:**

1. In `crates/strategy-runtime/src/nodes/filter.rs`, add a `compiled_filter: Program` field to the filter node struct.

2. In the filter node constructor, call `bytecode::compile(filter_expr, &slot_resolver)?` and store the result.

3. In the `filter(universe: &[UniverseEntry], world: &WorldState)` method, replace the per-entry expression evaluation with:
   ```rust
   universe.iter().filter(|entry| {
       // Load entry-specific slots into a local array
       let slots = entry.feature_slots_for(&world_state);
       bytecode::run(&self.compiled_filter, &slots, &world_state.current_bar) != 0.0
   }).collect()
   ```

4. Ensure the compilation happens exactly once per filter node instantiation, not per call.

**Acceptance test:**
- Write a unit test in `crates/strategy-runtime/src/bytecode.rs` that compiles `"feature('ema_7') > feature('ema_21')"` and runs it with a mock slot array. Verify the result is correct for both `ema_7 > ema_21` and `ema_7 <= ema_21` cases.
- Use `dhat-rs` or a `#[global_allocator]` counter in an integration test to verify zero heap allocations occur inside `bytecode::run` and `runtime::process_event` (excluding the initial compilation).
- All existing strategy evaluation tests must pass without modification.

## Overall Acceptance Criteria
- [ ] Zero heap allocations in `process_event` evaluation path (verified with allocation counter in tests)
- [ ] All condition expressions compiled exactly once at instance/node init (grep for `tokenize` in runtime.rs returns zero)
- [ ] Bytecode evaluator uses a fixed 32-element stack — no `Vec` growth per `run` call
- [ ] Universe filter node compiles its expressions at node creation time
- [ ] All existing strategy evaluation tests pass
- [ ] `cargo build --release` succeeds

## Files to Touch
- `crates/strategy-runtime/src/bytecode.rs` (new) — Op enum, Program type, run() function
- `crates/strategy-runtime/src/interpreter.rs` — add compile() function; transform eval_expr into compile_expr; restrict string-parse path to compile-only
- `crates/strategy-runtime/src/runtime.rs` — replace evaluate_condition() call with bytecode::run(); remove per-tick tokenize/parse
- `crates/strategy-runtime/src/nodes/filter.rs` — add compiled_filter field; compile at node init; run bytecode at filter time
- `crates/strategy-runtime/src/lib.rs` — export bytecode module
