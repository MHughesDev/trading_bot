//! Strategy definition validator — the single validator all three front doors target.
//!
//! All three front doors (JSON API, visual builder, MCP server) call `validate()`
//! before persisting or applying a definition. No door has a privileged bypass.

pub mod expressions;
pub mod risk;
pub mod schema;

use domain::strategy_def::{nodes::NodeKind, StrategyDefinition};

/// A structured validation error with a JSON-pointer–style path and a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// JSON-pointer–style path to the offending field (e.g. `"risk_overrides.max_position"`).
    pub path: String,
    /// Human-readable reason, suitable for agent self-correction.
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// A strategy definition that has passed all validation checks.
///
/// The private `_sealed` field ensures this can only be constructed via `validate()`.
#[derive(Debug, Clone)]
pub struct ValidatedDefinition {
    pub inner: StrategyDefinition,
    _sealed: (),
}

impl ValidatedDefinition {
    pub fn into_inner(self) -> StrategyDefinition {
        self.inner
    }
}

/// Validate a strategy definition against the frozen v1.0 rules.
///
/// Runs three passes in order:
/// 1. Schema — structural correctness (version, IDs, references).
/// 2. Expressions — condition expression syntax per the frozen grammar.
/// 3. Risk — tighten-only invariant against `GlobalRiskLimits::default()`.
///
/// All errors from all passes are collected before returning, so an agent
/// can see and fix all problems in one round trip.
pub fn validate(def: &StrategyDefinition) -> Result<ValidatedDefinition, Vec<ValidationError>> {
    let mut errors = Vec::new();

    errors.extend(schema::validate_schema(def));

    for node in &def.nodes {
        if let NodeKind::Condition { expr } = &node.kind {
            let path = format!("nodes[{}].expr", node.id);
            errors.extend(expressions::validate_expression(expr, &path));
        }
    }

    errors.extend(risk::validate_risk_overrides(&def.risk_overrides));

    if errors.is_empty() {
        Ok(ValidatedDefinition {
            inner: def.clone(),
            _sealed: (),
        })
    } else {
        Err(errors)
    }
}
