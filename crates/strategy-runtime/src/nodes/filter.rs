//! Filter node — keeps only instruments where the predicate expression is true.

use std::collections::HashMap;

use crate::bytecode;
use crate::interpreter::evaluate_condition;
use crate::nodes::UniverseEntry;

/// Retain only entries for which the pre-compiled `program` evaluates to true.
///
/// Callers should compile the expression once at pipeline-init time via
/// `bytecode::compile(expr)` and pass the resulting `Program` here so that
/// no re-parsing occurs per tick.
///
/// `feature(...)` calls in the program are resolved against each entry's
/// feature map; `bar(...)` is not available in universe-mode evaluation.
///
/// # Unknown excludes, and that is a choice made here
///
/// An entry whose predicate cannot be evaluated — a feature still warming, or
/// a history-bearing function, which has no bar clock in universe mode — is
/// dropped from the universe. That is a deliberate collapse of three values
/// into two, written at the call site rather than hidden inside the evaluator,
/// because a filter must return a list and "maybe" is not a member of one.
///
/// It is safe in the direction that matters: a universe is a candidate set, so
/// excluding an instrument nobody could assess narrows what gets traded rather
/// than widening it. It is *not* safe to copy this into signal evaluation,
/// where the same collapse silently removes entries — see
/// [`SignalEvaluator`](crate::interpreter::SignalEvaluator).
pub fn filter_compiled(
    universe: &[UniverseEntry],
    program: &bytecode::Program,
) -> Vec<UniverseEntry> {
    let empty_bars = HashMap::new();
    universe
        .iter()
        .filter(|entry| bytecode::run(program, &entry.features, &empty_bars).unknown_as_false())
        .cloned()
        .collect()
}

/// Retain only entries for which `expr` evaluates to true.
///
/// **Deprecated hot-path:** compiles the expression on every call.
/// Prefer `filter_compiled` when the same expression is evaluated more than once.
///
/// Falls back to the interpreter for any expression that fails to compile.
pub fn filter(universe: &[UniverseEntry], expr: &str) -> Vec<UniverseEntry> {
    let empty_bars = HashMap::new();
    match bytecode::compile(expr) {
        Ok(program) => filter_compiled(universe, &program),
        Err(_) => universe
            .iter()
            .filter(|entry| evaluate_condition(expr, &entry.features, &empty_bars))
            .cloned()
            .collect(),
    }
}
