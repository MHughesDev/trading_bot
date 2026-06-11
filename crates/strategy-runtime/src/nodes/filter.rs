//! Filter node — keeps only instruments where the predicate expression is true.

use std::collections::HashMap;

use crate::bytecode;
use crate::interpreter::evaluate_condition;
use crate::nodes::Universe;

/// Retain only entries for which `expr` evaluates to true.
///
/// The expression is compiled to bytecode once, then executed per entry.
/// Falls back to the interpreter for any expression that fails to compile.
///
/// `expr` may use `feature('name')` calls; `bar(...)` is not available in
/// universe-mode evaluation and will return false if referenced.
pub fn filter(universe: Universe, expr: &str) -> Universe {
    let empty_bars = HashMap::new();
    match bytecode::compile(expr) {
        Ok(program) => universe
            .into_iter()
            .filter(|entry| bytecode::run(&program, &entry.features, &empty_bars))
            .collect(),
        Err(_) => universe
            .into_iter()
            .filter(|entry| evaluate_condition(expr, &entry.features, &empty_bars))
            .collect(),
    }
}
