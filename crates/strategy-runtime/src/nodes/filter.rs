//! Filter node — keeps only instruments where the predicate expression is true.

use std::collections::HashMap;

use crate::interpreter::evaluate_condition;
use crate::nodes::Universe;

/// Retain only entries for which `expr` evaluates to true.
///
/// `expr` may use `feature('name')` calls; `bar(...)` is not available in
/// universe-mode evaluation and will return false if referenced.
pub fn filter(universe: Universe, expr: &str) -> Universe {
    let empty_bars = HashMap::new();
    universe
        .into_iter()
        .filter(|entry| evaluate_condition(expr, &entry.features, &empty_bars))
        .collect()
}
