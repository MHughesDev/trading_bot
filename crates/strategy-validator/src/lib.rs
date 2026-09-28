//! Strategy definition validator — the single validator all three front doors target.
//!
//! All three front doors (JSON API, visual builder, MCP server) call `validate()`
//! before persisting or applying a definition. No door has a privileged bypass.

pub mod exits;
pub mod expressions;
pub mod feature_names;
pub mod risk;
pub mod schema;
pub mod teach;
pub mod version;

use domain::strategy_def::{nodes::NodeKind, params, StrategyDefinition};

/// A structured validation error with a JSON-pointer–style path and a human message.
///
/// `fix` and `see` are AGENT-011 §5 (Set S task S-5.3). A rejection that only
/// says what is wrong leaves a small model three moves — guess another spelling,
/// drop the feature, drop the strategy — and no way to tell which, because the
/// message describes the error and not the edit. Each costs a turn.
///
/// They are **derived from `code`** rather than passed in. A caller that could
/// supply its own fix is a caller that can forget to, and the forgetting is
/// silent: the error still renders, just without the half that helps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The stable error identifier the teaching table keys on. Not for display —
    /// it is what a test, a renderer and a retry policy can all branch on, which
    /// a prose message cannot be.
    pub code: &'static str,
    /// JSON-pointer–style path to the offending field (e.g. `"risk_overrides.max_position"`).
    pub path: String,
    /// Human-readable reason, suitable for agent self-correction.
    pub message: String,
}

impl ValidationError {
    /// Build an error. `fix` and `see` follow from `code`.
    #[must_use]
    pub fn new(code: &'static str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            message: message.into(),
        }
    }

    /// The concrete edit that resolves this.
    ///
    /// **Derived, not stored.** A `fix` field would be a second copy of the
    /// table that a caller could fill in itself, forget to fill in, or leave
    /// behind when the table changed — and all three fail silently, because the
    /// error still renders, just without the half that helps.
    #[must_use]
    pub fn fix(&self) -> Option<&'static str> {
        teach::teaching(self.code).map(|t| t.fix)
    }

    /// Guide anchor for the rule behind this, e.g. `guide#exits.units`.
    #[must_use]
    pub fn see(&self) -> Option<&'static str> {
        teach::teaching(self.code).and_then(|t| t.see)
    }

    /// A worked example for this error's clause, for the caller that has now
    /// been rejected three times on the same path (AGENT-011 §5).
    ///
    /// Offered by the caller rather than attached here, because "three times" is
    /// run-scoped state and this crate is a pure function of one definition.
    #[must_use]
    pub fn example(&self) -> Option<&'static str> {
        teach::example_for_path(&self.path)
    }

    /// The error as the API and the MCP server return it.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code,
            "path": self.path,
            "message": self.message,
            "fix": self.fix(),
            "see": self.see(),
        })
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// A strategy definition that has passed all validation checks.
///
/// `#[non_exhaustive]` ensures this can only be constructed via `validate()`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ValidatedDefinition {
    pub inner: StrategyDefinition,
}

impl ValidatedDefinition {
    pub fn into_inner(self) -> StrategyDefinition {
        self.inner
    }
}

/// Validate a strategy definition against the frozen v1.0 rules.
///
/// Runs six passes in order:
/// 1. Schema — structural correctness (version, IDs, references).
/// 2. Parameters (v1.2) — declarations are well-formed, every `param('x')` /
///    `{{x}}` reference resolves, cross-parameter constraints hold on defaults.
/// 3. Feature names — every name an `inputs` declaration asks for resolves in
///    the engine registry, checked on the definition **materialized with its
///    defaults**, so a swept period (`ema_{{fast}}`) is read as the name the
///    engine will be asked for rather than as a misspelling.
/// 4. Expressions — condition expression syntax per the frozen grammar, checked
///    on the same materialized definition so the grammar itself never has to
///    know about parameters.
/// 5. Risk — tighten-only invariant against `GlobalRiskLimits::default()`.
/// 6. Exits (v1.3) — the `exits` block is present, non-empty when it claims to
///    be, and its rules are in range. See [`exits`] for why silence is refused.
///
/// All errors from all passes are collected before returning, so an agent
/// can see and fix all problems in one round trip.
pub fn validate(def: &StrategyDefinition) -> Result<ValidatedDefinition, Vec<ValidationError>> {
    let mut errors = Vec::new();

    errors.extend(schema::validate_schema(def));

    // Parameters: declarations, references, constraints — then materialize
    // defaults so the feature-name and expression passes see plain literals.
    if let Err(e) = params::validate_declarations(def) {
        errors.push(ValidationError {
            code: "parameter_out_of_range",
            path: "parameters".into(),
            message: e.to_string(),
        });
    }
    let substituted = match params::materialize(def, &params::ParamValues::new()) {
        Ok(m) => Some(m),
        Err(e) => {
            errors.push(ValidationError {
                code: "parameter_out_of_range",
                path: "parameters".into(),
                message: e.to_string(),
            });
            None
        }
    };

    // Declared features must resolve. Before this pass a typo validated cleanly
    // and failed inside `features::runtime::feature` during the run — which for
    // an agent is after a trial was spent, so the cheapest error in the system
    // was charged at its most expensive point.
    //
    // **Read on the substituted copy, not on the definition as written.** A
    // swept indicator period is stored as `ema_{{fast}}` (v1.2 interpolation,
    // which is what `strategy-lang` emits for `input x = ema(close, fast)`), and
    // `features::resolve_feature` has never heard of it — so this pass refused
    // every strategy a sweep can run, and offered `ema_20` as the repair. An
    // author who takes that advice deletes their own parameter, the document
    // then validates, and the suggestion has confirmed itself by removing the
    // thing it could not read. Substituting the defaults first is what the
    // expression pass below has always done, for the same reason.
    //
    // Skipped entirely when substitution failed: the parameter error pushed
    // above is the answer, and reporting `unknown_feature` for `ema_{{nope}}` on
    // top of it would send the author to the `inputs` clause to fix something
    // that is wrong in the `parameters` clause.
    if let Some(m) = &substituted {
        errors.extend(feature_names::check(m));
    }

    // The expression pass keeps its fallback to the definition as written: a
    // malformed expression is worth reporting even when the parameters could not
    // be resolved, because the two are independent mistakes.
    let materialized = substituted.unwrap_or_else(|| def.clone());

    for node in &materialized.nodes {
        if let NodeKind::Condition { expr } = &node.kind {
            let path = format!("nodes[{}].expr", node.id);
            errors.extend(expressions::validate_expression(expr, &path));
        }
    }

    errors.extend(risk::validate_risk_overrides(&def.risk_overrides));
    errors.extend(exits::validate_exits(def));

    if errors.is_empty() {
        Ok(ValidatedDefinition { inner: def.clone() })
    } else {
        Err(errors)
    }
}
