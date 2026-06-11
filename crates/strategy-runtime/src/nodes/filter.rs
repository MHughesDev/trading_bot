//! Filter node — keeps only instruments where the predicate expression is true.

use std::collections::HashMap;

use crate::bytecode;
use crate::interpreter::evaluate_condition;
use crate::nodes::Universe;

/// Retain only entries for which the pre-compiled `program` evaluates to true.
///
/// Callers should compile the expression once at pipeline-init time via
/// `bytecode::compile(expr)` and pass the resulting `Program` here so that
/// no re-parsing occurs per tick.
///
/// `feature(...)` calls in the program are resolved against each entry's
/// feature map; `bar(...)` is not available in universe-mode evaluation and
/// will return false if referenced.
pub fn filter_compiled(universe: Universe, program: &bytecode::Program) -> Universe {
    let empty_bars = HashMap::new();
    universe
        .into_iter()
        .filter(|entry| bytecode::run(program, &entry.features, &empty_bars))
        .collect()
}

/// Retain only entries for which `expr` evaluates to true.
///
/// **Deprecated hot-path:** compiles the expression on every call.
/// Prefer `filter_compiled` when the same expression is evaluated more than once.
///
/// Falls back to the interpreter for any expression that fails to compile.
pub fn filter(universe: Universe, expr: &str) -> Universe {
    let empty_bars = HashMap::new();
    match bytecode::compile(expr) {
        Ok(program) => filter_compiled(universe, &program),
        Err(_) => universe
            .into_iter()
            .filter(|entry| evaluate_condition(expr, &entry.features, &empty_bars))
            .collect(),
    }
}
