//! [`SearchSpace`] — the sweepable parameters of one strategy definition,
//! encoded to and from the unit cube so samplers are dimension-agnostic.

use std::collections::BTreeMap;

use backtest::run::ParamMap;
use domain::strategy_def::params::{self, ParamSpec, Scale};
use domain::strategy_def::StrategyDefinition;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Agent-supplied narrowing of one parameter. Only tightening is legal: a
/// range outside the declaration is rejected, never silently clamped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Narrow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SpaceError {
    #[error("strategy declares no `parameters` block — nothing to sweep")]
    NoParameters,
    #[error("narrowing names undeclared parameter '{0}'")]
    UnknownParam(String),
    #[error("narrowing for '{name}' widens the declaration ({reason})")]
    Widens { name: String, reason: String },
    #[error("narrowing for '{0}' is empty")]
    Empty(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DimKind {
    Int { min: i64, max: i64, step: i64 },
    Float { min: f64, max: f64, log: bool },
    Enum { choices: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dim {
    pub name: String,
    pub kind: DimKind,
}

impl Dim {
    /// Whether the dimension is numeric (surfaces are binned on it).
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        !matches!(self.kind, DimKind::Enum { .. })
    }

    /// Unit-cube coordinate of a concrete value.
    #[must_use]
    pub fn encode(&self, v: &Value) -> f64 {
        match &self.kind {
            DimKind::Int { min, max, .. } => {
                let x = v.as_f64().unwrap_or(*min as f64);
                if max == min {
                    0.0
                } else {
                    ((x - *min as f64) / (*max - *min) as f64).clamp(0.0, 1.0)
                }
            }
            DimKind::Float { min, max, log } => {
                let x = v.as_f64().unwrap_or(*min);
                if *log {
                    let (lo, hi) = (min.ln(), max.ln());
                    if hi > lo {
                        ((x.max(*min).ln() - lo) / (hi - lo)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                } else if max > min {
                    ((x - min) / (max - min)).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            DimKind::Enum { choices } => {
                let idx = v
                    .as_str()
                    .and_then(|s| choices.iter().position(|c| c == s))
                    .unwrap_or(0);
                if choices.len() <= 1 {
                    0.0
                } else {
                    idx as f64 / (choices.len() - 1) as f64
                }
            }
        }
    }

    /// Concrete value at a unit-cube coordinate (ints snapped to `step`).
    #[must_use]
    pub fn decode(&self, u: f64) -> Value {
        let u = u.clamp(0.0, 1.0);
        match &self.kind {
            DimKind::Int { min, max, step } => {
                let raw = *min as f64 + u * (*max - *min) as f64;
                let snapped = min + ((raw - *min as f64) / *step as f64).round() as i64 * step;
                Value::from(snapped.clamp(*min, *max))
            }
            DimKind::Float { min, max, log } => {
                let x = if *log {
                    (min.ln() + u * (max.ln() - min.ln())).exp()
                } else {
                    min + u * (max - min)
                };
                serde_json::Number::from_f64(x.clamp(*min, *max)).map_or(Value::Null, Value::Number)
            }
            DimKind::Enum { choices } => {
                let idx = (u * (choices.len().saturating_sub(1)) as f64).round() as usize;
                Value::String(choices[idx.min(choices.len() - 1)].clone())
            }
        }
    }

    /// Human rendering of a coordinate (for surface text).
    #[must_use]
    pub fn render(&self, u: f64) -> String {
        params::literal(&self.decode(u))
    }
}

/// The sweepable space of one definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchSpace {
    pub dims: Vec<Dim>,
    /// Kept so feasibility (cross-parameter constraints) can be checked
    /// without the caller re-supplying the definition.
    definition: StrategyDefinition,
}

impl SearchSpace {
    /// Build from the definition's `parameters`, applying `narrowing`.
    ///
    /// # Errors
    /// [`SpaceError`] if there is nothing to sweep or a narrowing is illegal.
    pub fn from_definition(
        def: &StrategyDefinition,
        narrowing: &BTreeMap<String, Narrow>,
    ) -> Result<Self, SpaceError> {
        if def.parameters.is_empty() {
            return Err(SpaceError::NoParameters);
        }
        if let Some(unknown) = narrowing.keys().find(|k| !def.parameters.contains_key(*k)) {
            return Err(SpaceError::UnknownParam(unknown.clone()));
        }
        let mut dims = Vec::with_capacity(def.parameters.len());
        for (name, spec) in &def.parameters {
            let n = narrowing.get(name);
            let kind = match spec {
                ParamSpec::Int { min, max, step, .. } => {
                    let lo = n.and_then(|n| n.min).map_or(*min, |v| v.ceil() as i64);
                    let hi = n.and_then(|n| n.max).map_or(*max, |v| v.floor() as i64);
                    if lo < *min || hi > *max {
                        return Err(SpaceError::Widens {
                            name: name.clone(),
                            reason: format!("declared [{min}, {max}], asked [{lo}, {hi}]"),
                        });
                    }
                    if lo > hi {
                        return Err(SpaceError::Empty(name.clone()));
                    }
                    DimKind::Int {
                        min: lo,
                        max: hi,
                        step: *step,
                    }
                }
                ParamSpec::Float {
                    min, max, scale, ..
                } => {
                    let lo = n.and_then(|n| n.min).unwrap_or(*min);
                    let hi = n.and_then(|n| n.max).unwrap_or(*max);
                    if lo < *min || hi > *max {
                        return Err(SpaceError::Widens {
                            name: name.clone(),
                            reason: format!("declared [{min}, {max}], asked [{lo}, {hi}]"),
                        });
                    }
                    if lo > hi {
                        return Err(SpaceError::Empty(name.clone()));
                    }
                    DimKind::Float {
                        min: lo,
                        max: hi,
                        log: *scale == Scale::Log,
                    }
                }
                ParamSpec::Enum { choices, .. } => {
                    let chosen = match n.and_then(|n| n.choices.clone()) {
                        Some(c) => {
                            if let Some(bad) = c.iter().find(|x| !choices.contains(x)) {
                                return Err(SpaceError::Widens {
                                    name: name.clone(),
                                    reason: format!("'{bad}' is not a declared choice"),
                                });
                            }
                            if c.is_empty() {
                                return Err(SpaceError::Empty(name.clone()));
                            }
                            c
                        }
                        None => choices.clone(),
                    };
                    DimKind::Enum { choices: chosen }
                }
            };
            dims.push(Dim {
                name: name.clone(),
                kind,
            });
        }
        Ok(Self {
            dims,
            definition: def.clone(),
        })
    }

    #[must_use]
    pub fn n_dims(&self) -> usize {
        self.dims.len()
    }

    #[must_use]
    pub fn dim(&self, name: &str) -> Option<&Dim> {
        self.dims.iter().find(|d| d.name == name)
    }

    /// Unit-cube coordinates of a parameter map (missing → declaration default).
    #[must_use]
    pub fn encode(&self, p: &ParamMap) -> Vec<f64> {
        self.dims
            .iter()
            .map(|d| {
                let v = p
                    .get(&d.name)
                    .cloned()
                    .unwrap_or_else(|| self.definition.parameters[&d.name].default_value());
                d.encode(&v)
            })
            .collect()
    }

    /// Parameter map at unit-cube coordinates.
    #[must_use]
    pub fn decode(&self, u: &[f64]) -> ParamMap {
        self.dims
            .iter()
            .zip(u)
            .map(|(d, &x)| (d.name.clone(), d.decode(x)))
            .collect()
    }

    /// The declaration defaults as a point.
    #[must_use]
    pub fn default_point(&self) -> ParamMap {
        self.dims
            .iter()
            .map(|d| {
                (
                    d.name.clone(),
                    self.definition.parameters[&d.name].default_value(),
                )
            })
            .collect()
    }

    /// True if the point satisfies every declaration and cross-parameter
    /// constraint. Infeasible points are re-drawn and never become Runs.
    #[must_use]
    pub fn feasible(&self, p: &ParamMap) -> bool {
        params::resolve(&self.definition, p).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn fixture() -> StrategyDefinition {
        serde_json::from_value(json!({
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "parameters": {
                "fast": { "type": "int", "default": 7, "min": 3, "max": 50 },
                "slow": { "type": "int", "default": 21, "min": 10, "max": 200 },
                "gate": { "type": "float", "default": 0.01, "min": 0.001, "max": 0.1, "scale": "log" },
                "exit": { "type": "enum", "default": "trail", "choices": ["trail", "fixed", "time"] }
            },
            "constraints": ["param('fast') < param('slow')"],
            "inputs": [], "nodes": [], "actions": []
        }))
        .unwrap()
    }

    #[test]
    fn round_trips_through_unit_cube() {
        let s = SearchSpace::from_definition(&fixture(), &BTreeMap::new()).unwrap();
        assert_eq!(s.n_dims(), 4);
        let p = s.default_point();
        let u = s.encode(&p);
        let back = s.decode(&u);
        assert_eq!(back["fast"], json!(7));
        assert_eq!(back["slow"], json!(21));
        assert_eq!(back["exit"], json!("trail"));
        assert!((back["gate"].as_f64().unwrap() - 0.01).abs() < 1e-9);
        assert!(s.feasible(&p));
    }

    #[test]
    fn narrowing_tightens_but_cannot_widen() {
        let mut n = BTreeMap::new();
        n.insert(
            "fast".to_string(),
            Narrow {
                min: Some(8.0),
                max: Some(30.0),
                choices: None,
            },
        );
        let s = SearchSpace::from_definition(&fixture(), &n).unwrap();
        assert_eq!(
            s.dim("fast").unwrap().kind,
            DimKind::Int {
                min: 8,
                max: 30,
                step: 1
            }
        );
        n.insert(
            "slow".to_string(),
            Narrow {
                min: None,
                max: Some(500.0),
                choices: None,
            },
        );
        assert!(matches!(
            SearchSpace::from_definition(&fixture(), &n),
            Err(SpaceError::Widens { .. })
        ));
    }

    #[test]
    fn constraint_makes_points_infeasible() {
        let s = SearchSpace::from_definition(&fixture(), &BTreeMap::new()).unwrap();
        let mut p = s.default_point();
        p.insert("fast".into(), json!(40));
        p.insert("slow".into(), json!(12));
        assert!(!s.feasible(&p));
    }

    #[test]
    fn no_parameters_is_an_error() {
        let mut d = fixture();
        d.parameters.clear();
        d.constraints.clear();
        assert_eq!(
            SearchSpace::from_definition(&d, &BTreeMap::new()),
            Err(SpaceError::NoParameters)
        );
    }
}
