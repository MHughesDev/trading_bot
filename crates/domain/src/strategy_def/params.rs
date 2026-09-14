//! Typed strategy parameters (v1.2, additive).
//!
//! A definition may declare a `parameters` block. Expressions reference a
//! parameter with `param('name')` (same call syntax as `feature`/`bar`), and any
//! string — expression or feature name — may embed `{{name}}`. Neither form is
//! understood by the runtime: [`materialize`] substitutes literals *before*
//! validation and execution, so every downstream consumer (validator grammar,
//! requirements derivation, simulator, live runtime) sees the frozen v1.0
//! grammar unchanged.
//!
//! ```jsonc
//! "parameters": {
//!   "fast": { "type": "int",   "default": 12,   "min": 5,     "max": 50 },
//!   "gate": { "type": "float", "default": 0.02, "min": 0.005, "max": 0.08, "scale": "log" },
//!   "exit": { "type": "enum",  "default": "trail", "choices": ["trail", "fixed"] }
//! },
//! "nodes": [
//!   { "id": "n1", "type": "condition", "expr": "feature('ema_{{fast}}') > feature('ema_{{slow}}')" }
//! ]
//! ```
//!
//! `RunConfig.params` (the backtest suite's `ParamMap`) is the only override
//! path; nothing edits expression strings by hand.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::nodes::NodeKind;
use super::StrategyDefinition;

/// Raw override values keyed by parameter name (mirrors the suite's `ParamMap`).
pub type ParamValues = BTreeMap<String, Value>;

/// How a numeric range is traversed by a sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    #[default]
    Linear,
    Log,
}

fn one() -> i64 {
    1
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ParamSpec {
    Int {
        default: i64,
        min: i64,
        max: i64,
        #[serde(default = "one")]
        step: i64,
    },
    Float {
        default: f64,
        min: f64,
        max: f64,
        #[serde(default)]
        scale: Scale,
    },
    Enum {
        default: String,
        choices: Vec<String>,
    },
}

/// A cross-parameter constraint, e.g. `param('fast') < param('slow')`.
///
/// Grammar (deliberately tiny): `operand cmp operand` where an operand is a
/// `param('name')` reference or a decimal literal and `cmp` is one of
/// `< <= > >= == !=`.
pub type Constraint = String;

/// Why a parameter block or override set is invalid.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ParamError {
    #[error("parameter '{0}' is referenced but not declared")]
    Undeclared(String),
    #[error("parameter '{name}' value {value} is outside [{min}, {max}]")]
    OutOfRange {
        name: String,
        value: String,
        min: String,
        max: String,
    },
    #[error("parameter '{name}' expects {expected}, got {got}")]
    WrongType {
        name: String,
        expected: &'static str,
        got: String,
    },
    #[error("parameter '{name}' declaration invalid: {reason}")]
    BadSpec { name: String, reason: String },
    #[error("constraint '{constraint}' violated")]
    ConstraintViolated { constraint: String },
    #[error("constraint '{constraint}' malformed: {reason}")]
    BadConstraint { constraint: String, reason: String },
}

impl ParamSpec {
    /// Check the declaration itself (ranges, defaults, choices).
    pub fn validate_spec(&self, name: &str) -> Result<(), ParamError> {
        let bad = |reason: String| ParamError::BadSpec {
            name: name.to_string(),
            reason,
        };
        match self {
            ParamSpec::Int {
                default,
                min,
                max,
                step,
            } => {
                if min > max {
                    return Err(bad(format!("min {min} > max {max}")));
                }
                if *step <= 0 {
                    return Err(bad(format!("step must be positive, got {step}")));
                }
                if default < min || default > max {
                    return Err(bad(format!("default {default} outside [{min}, {max}]")));
                }
            }
            ParamSpec::Float {
                default,
                min,
                max,
                scale,
            } => {
                if !(min.is_finite() && max.is_finite() && default.is_finite()) {
                    return Err(bad("min/max/default must be finite".into()));
                }
                if min > max {
                    return Err(bad(format!("min {min} > max {max}")));
                }
                if default < min || default > max {
                    return Err(bad(format!("default {default} outside [{min}, {max}]")));
                }
                if *scale == Scale::Log && *min <= 0.0 {
                    return Err(bad("log scale requires min > 0".into()));
                }
            }
            ParamSpec::Enum { default, choices } => {
                if choices.is_empty() {
                    return Err(bad("enum needs at least one choice".into()));
                }
                if !choices.contains(default) {
                    return Err(bad(format!("default '{default}' not among choices")));
                }
            }
        }
        Ok(())
    }

    /// The declared default as a JSON value.
    #[must_use]
    pub fn default_value(&self) -> Value {
        match self {
            ParamSpec::Int { default, .. } => Value::from(*default),
            ParamSpec::Float { default, .. } => {
                serde_json::Number::from_f64(*default).map_or(Value::Null, Value::Number)
            }
            ParamSpec::Enum { default, .. } => Value::String(default.clone()),
        }
    }

    /// Check an override against this declaration, returning the normalised
    /// value (ints coerced from integral floats, floats from ints).
    pub fn check(&self, name: &str, v: &Value) -> Result<Value, ParamError> {
        let wrong = |expected: &'static str| ParamError::WrongType {
            name: name.to_string(),
            expected,
            got: v.to_string(),
        };
        match self {
            ParamSpec::Int { min, max, .. } => {
                let i = match v {
                    Value::Number(n) => n
                        .as_i64()
                        .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
                        .ok_or_else(|| wrong("int"))?,
                    _ => return Err(wrong("int")),
                };
                if i < *min || i > *max {
                    return Err(ParamError::OutOfRange {
                        name: name.to_string(),
                        value: i.to_string(),
                        min: min.to_string(),
                        max: max.to_string(),
                    });
                }
                Ok(Value::from(i))
            }
            ParamSpec::Float { min, max, .. } => {
                let f = v.as_f64().ok_or_else(|| wrong("float"))?;
                if !f.is_finite() || f < *min || f > *max {
                    return Err(ParamError::OutOfRange {
                        name: name.to_string(),
                        value: f.to_string(),
                        min: min.to_string(),
                        max: max.to_string(),
                    });
                }
                Ok(serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number))
            }
            ParamSpec::Enum { choices, .. } => {
                let s = v.as_str().ok_or_else(|| wrong("enum string"))?;
                if !choices.iter().any(|c| c == s) {
                    return Err(ParamError::OutOfRange {
                        name: name.to_string(),
                        value: s.to_string(),
                        min: choices.first().cloned().unwrap_or_default(),
                        max: choices.last().cloned().unwrap_or_default(),
                    });
                }
                Ok(Value::String(s.to_string()))
            }
        }
    }
}

/// Render a checked value as a grammar-legal literal for substitution.
///
/// Ints render bare (`12`); floats as a decimal literal with no exponent
/// (`0.02`, `-1.5`); enums as their raw string (meant for `{{name}}` in feature
/// names — inside an expression the validator will reject a bare word, which
/// is the right outcome).
#[must_use]
pub fn literal(v: &Value) -> String {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return i.to_string();
            }
            let f = n.as_f64().unwrap_or(0.0);
            decimal_literal(f)
        }
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Fixed-point rendering: never scientific notation, at least one fractional
/// digit so the token is unambiguous as a decimal.
fn decimal_literal(f: f64) -> String {
    let s = format!("{f:.12}");
    let s = s.trim_end_matches('0');
    if s.ends_with('.') {
        format!("{s}0")
    } else {
        s.to_string()
    }
}

/// Validate every declaration in the block.
pub fn validate_declarations(def: &StrategyDefinition) -> Result<(), ParamError> {
    for (name, spec) in &def.parameters {
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ParamError::BadSpec {
                name: name.clone(),
                reason: "name must be [A-Za-z0-9_]+".into(),
            });
        }
        spec.validate_spec(name)?;
    }
    for c in &def.constraints {
        parse_constraint(c)?;
    }
    Ok(())
}

/// Defaults overlaid with `overrides`, every value checked, undeclared
/// overrides rejected, cross-parameter constraints enforced.
pub fn resolve(
    def: &StrategyDefinition,
    overrides: &ParamValues,
) -> Result<ParamValues, ParamError> {
    let mut out = ParamValues::new();
    for (name, spec) in &def.parameters {
        let v = match overrides.get(name) {
            Some(v) => spec.check(name, v)?,
            None => spec.default_value(),
        };
        out.insert(name.clone(), v);
    }
    if let Some(extra) = overrides.keys().find(|k| !def.parameters.contains_key(*k)) {
        return Err(ParamError::Undeclared(extra.clone()));
    }
    for c in &def.constraints {
        if !eval_constraint(c, &out)? {
            return Err(ParamError::ConstraintViolated {
                constraint: c.clone(),
            });
        }
    }
    Ok(out)
}

/// Substitute every `param('x')` and `{{x}}` in the definition with the
/// resolved literal. The returned definition contains no parameter references
/// and is what every executor runs.
pub fn materialize(
    def: &StrategyDefinition,
    overrides: &ParamValues,
) -> Result<StrategyDefinition, ParamError> {
    let values = resolve(def, overrides)?;
    let mut out = def.clone();
    for input in &mut out.inputs {
        for f in &mut input.features {
            *f = substitute(f, &values)?;
        }
    }
    for node in &mut out.nodes {
        match &mut node.kind {
            NodeKind::Condition { expr } | NodeKind::Filter { expr, .. } => {
                *expr = substitute(expr, &values)?;
            }
            _ => {}
        }
    }
    Ok(out)
}

/// True if any string in the definition still references a parameter.
#[must_use]
pub fn has_references(def: &StrategyDefinition) -> bool {
    let hit = |s: &str| s.contains("param(") || s.contains("{{");
    def.inputs.iter().any(|i| i.features.iter().any(|f| hit(f)))
        || def.nodes.iter().any(|n| match &n.kind {
            NodeKind::Condition { expr } | NodeKind::Filter { expr, .. } => hit(expr),
            _ => false,
        })
}

/// Replace `param('name')` and `{{name}}` occurrences in `s`.
fn substitute(s: &str, values: &ParamValues) -> Result<String, ParamError> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let p = rest.find("param(");
        let b = rest.find("{{");
        let (idx, kind) = match (p, b) {
            (None, None) => {
                out.push_str(rest);
                return Ok(out);
            }
            (Some(p), None) => (p, 'p'),
            (None, Some(b)) => (b, 'b'),
            (Some(p), Some(b)) => {
                if p < b {
                    (p, 'p')
                } else {
                    (b, 'b')
                }
            }
        };
        out.push_str(&rest[..idx]);
        let after = &rest[idx..];
        let (name, consumed) = if kind == 'p' {
            // param('name')  — tolerate spaces and either quote.
            let inner = &after["param(".len()..];
            let close = inner
                .find(')')
                .ok_or_else(|| ParamError::Undeclared(after.chars().take(24).collect()))?;
            let raw = inner[..close]
                .trim()
                .trim_matches(|c| c == '\'' || c == '"');
            (raw.to_string(), "param(".len() + close + 1)
        } else {
            let inner = &after[2..];
            let close = inner
                .find("}}")
                .ok_or_else(|| ParamError::Undeclared(after.chars().take(24).collect()))?;
            (inner[..close].trim().to_string(), 2 + close + 2)
        };
        let v = values
            .get(&name)
            .ok_or_else(|| ParamError::Undeclared(name.clone()))?;
        out.push_str(&literal(v));
        rest = &after[consumed..];
    }
}

// ── constraints ──────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum Operand {
    Param(String),
    Num(f64),
}

fn parse_operand(tok: &str) -> Result<Operand, String> {
    let t = tok.trim();
    if let Some(inner) = t.strip_prefix("param(").and_then(|r| r.strip_suffix(')')) {
        let name = inner.trim().trim_matches(|c| c == '\'' || c == '"');
        if name.is_empty() {
            return Err("empty param name".into());
        }
        return Ok(Operand::Param(name.to_string()));
    }
    t.parse::<f64>()
        .map(Operand::Num)
        .map_err(|_| format!("operand '{t}' is neither param(...) nor a number"))
}

fn parse_constraint(c: &str) -> Result<(Operand, &'static str, Operand), ParamError> {
    let bad = |reason: String| ParamError::BadConstraint {
        constraint: c.to_string(),
        reason,
    };
    // Longest operators first so `<=` is not split as `<`.
    for op in ["<=", ">=", "==", "!=", "<", ">"] {
        if let Some((l, r)) = c.split_once(op) {
            let lhs = parse_operand(l).map_err(bad)?;
            let rhs = parse_operand(r).map_err(bad)?;
            return Ok((lhs, op, rhs));
        }
    }
    Err(bad("no comparison operator found".into()))
}

fn operand_value(o: &Operand, values: &ParamValues) -> Result<f64, ParamError> {
    match o {
        Operand::Num(n) => Ok(*n),
        Operand::Param(name) => values
            .get(name)
            .and_then(Value::as_f64)
            .ok_or_else(|| ParamError::Undeclared(name.clone())),
    }
}

fn eval_constraint(c: &str, values: &ParamValues) -> Result<bool, ParamError> {
    let (l, op, r) = parse_constraint(c)?;
    let (a, b) = (operand_value(&l, values)?, operand_value(&r, values)?);
    Ok(match op {
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        "==" => (a - b).abs() < 1e-12,
        "!=" => (a - b).abs() >= 1e-12,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def_with_params() -> StrategyDefinition {
        serde_json::from_value(json!({
            "strategy_id": "ema_cross_p",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "parameters": {
                "fast": { "type": "int", "default": 7, "min": 3, "max": 50 },
                "slow": { "type": "int", "default": 21, "min": 10, "max": 200 },
                "gate": { "type": "float", "default": 0.5, "min": 0.0, "max": 5.0 }
            },
            "constraints": ["param('fast') < param('slow')"],
            "inputs": [
                { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
                { "lane": "features.technical", "instrument": "$bound_at_init",
                  "features": ["ema_{{fast}}", "ema_{{slow}}"] }
            ],
            "nodes": [
                { "id": "n1", "type": "condition",
                  "expr": "feature('ema_{{fast}}') - feature('ema_{{slow}}') > param('gate')" },
                { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
            ],
            "actions": []
        }))
        .expect("fixture deserializes")
    }

    #[test]
    fn materializes_defaults_into_grammar_legal_literals() {
        let d = materialize(&def_with_params(), &ParamValues::new()).unwrap();
        assert_eq!(d.inputs[1].features, vec!["ema_7", "ema_21"]);
        let NodeKind::Condition { expr } = &d.nodes[0].kind else {
            panic!("condition")
        };
        assert_eq!(expr, "feature('ema_7') - feature('ema_21') > 0.5");
        assert!(!has_references(&d));
    }

    #[test]
    fn overrides_apply_and_are_range_checked() {
        let mut o = ParamValues::new();
        o.insert("fast".into(), json!(12));
        o.insert("gate".into(), json!(0.02));
        let d = materialize(&def_with_params(), &o).unwrap();
        assert_eq!(d.inputs[1].features[0], "ema_12");
        let NodeKind::Condition { expr } = &d.nodes[0].kind else {
            panic!()
        };
        assert!(expr.ends_with("> 0.02"), "{expr}");

        let mut bad = ParamValues::new();
        bad.insert("fast".into(), json!(999));
        assert!(matches!(
            materialize(&def_with_params(), &bad),
            Err(ParamError::OutOfRange { .. })
        ));
    }

    #[test]
    fn undeclared_override_and_reference_are_rejected() {
        let mut o = ParamValues::new();
        o.insert("nope".into(), json!(1));
        assert_eq!(
            resolve(&def_with_params(), &o),
            Err(ParamError::Undeclared("nope".into()))
        );

        let mut d = def_with_params();
        d.nodes[0].kind = NodeKind::Condition {
            expr: "param('ghost') > 1".into(),
        };
        assert_eq!(
            materialize(&d, &ParamValues::new()),
            Err(ParamError::Undeclared("ghost".into()))
        );
    }

    #[test]
    fn constraints_are_enforced() {
        let mut o = ParamValues::new();
        o.insert("fast".into(), json!(30));
        o.insert("slow".into(), json!(20));
        assert!(matches!(
            resolve(&def_with_params(), &o),
            Err(ParamError::ConstraintViolated { .. })
        ));
    }

    #[test]
    fn float_literals_never_use_exponents() {
        assert_eq!(decimal_literal(0.00001), "0.00001");
        assert_eq!(decimal_literal(12.0), "12.0");
        assert_eq!(decimal_literal(-1.5), "-1.5");
        assert_eq!(literal(&json!(7)), "7");
    }

    #[test]
    fn definitions_without_parameters_are_untouched() {
        let mut d = def_with_params();
        d.parameters.clear();
        d.constraints.clear();
        d.inputs[1].features = vec!["ema_7".into()];
        d.nodes[0].kind = NodeKind::Condition {
            expr: "feature('ema_7') > 1".into(),
        };
        let m = materialize(&d, &ParamValues::new()).unwrap();
        assert_eq!(m, d);
    }
}
