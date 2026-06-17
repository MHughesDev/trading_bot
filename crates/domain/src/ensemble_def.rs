//! Ensemble definition format (ADR-0018, I-4.2).
//!
//! An ensemble composes a roster of trained model versions and produces a single
//! calibrated distributional forecast.  The definition is immutable once a
//! version is registered (same lifecycle as `ModelDefinition`).

use serde::{Deserialize, Serialize};

pub const ENSEMBLE_SCHEMA_VERSION: &str = "1.0";

// ── Sub-types ─────────────────────────────────────────────────────────────────

/// One member of the ensemble roster.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterMember {
    /// Registry model ID (e.g. `"mdl_abc123"`).
    pub model_ref: String,
    /// Alias to resolve at combine time (`"production"` | `"candidate"` | version number).
    #[serde(default = "default_alias")]
    pub alias: String,
}

fn default_alias() -> String {
    "production".to_string()
}

/// Calibration plan for the ensemble output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnsembleCalibrationSpec {
    /// `"conformal"` (default, adaptive ACI) | `"none"`.
    #[serde(default = "default_ens_cal_method")]
    pub method: String,
    /// Use adaptive conformal inference (ACI) — tracks drift.
    #[serde(default = "default_adaptive")]
    pub adaptive: bool,
    /// Role used to fit conformal residuals: `"cal"` (default) | `"val"`.
    #[serde(default = "default_ens_cal_fit_on")]
    pub fit_on: String,
}

fn default_ens_cal_method() -> String {
    "conformal".to_string()
}
fn default_adaptive() -> bool {
    true
}
fn default_ens_cal_fit_on() -> String {
    "cal".to_string()
}

impl Default for EnsembleCalibrationSpec {
    fn default() -> Self {
        Self {
            method: default_ens_cal_method(),
            adaptive: default_adaptive(),
            fit_on: default_ens_cal_fit_on(),
        }
    }
}

// ── EnsembleDefinition ────────────────────────────────────────────────────────

/// Ensemble definition — the frozen spec serialized into `ensembles.definition_json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnsembleDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,

    /// Non-empty list of member models.
    pub roster: Vec<RosterMember>,

    /// Combiner ID: `"linear_opinion_pool"` | `"crps_weighted"` | `"stacking"`.
    #[serde(default = "default_combiner")]
    pub combiner: String,

    /// Minimum weight fraction for any roster member (no member below this).
    /// Feasibility: `weight_floor * roster.len() ≤ 1.0`.
    #[serde(default = "default_weight_floor")]
    pub weight_floor: f64,

    /// Temperature for weight sharpening/softening.  1.0 = identity; < 1 sharpens.
    #[serde(default = "default_temperature")]
    pub temperature: f64,

    /// Calibration plan.  Defaults to adaptive conformal on the `cal` role.
    #[serde(default)]
    pub calibration: EnsembleCalibrationSpec,
}

fn default_schema_version() -> String {
    ENSEMBLE_SCHEMA_VERSION.to_string()
}
fn default_combiner() -> String {
    "linear_opinion_pool".to_string()
}
fn default_weight_floor() -> f64 {
    0.05
}
fn default_temperature() -> f64 {
    1.0
}

// ── Validation ────────────────────────────────────────────────────────────────

/// A validation error on an ensemble definition field.
#[derive(Debug, Clone)]
pub struct EnsembleValidationError {
    pub path: String,
    pub message: String,
}

/// Validate an `EnsembleDefinition`.  Returns `Ok(())` or a non-empty list of errors.
pub fn validate_ensemble(def: &EnsembleDefinition) -> Result<(), Vec<EnsembleValidationError>> {
    let mut errs: Vec<EnsembleValidationError> = Vec::new();

    if def.roster.is_empty() {
        errs.push(EnsembleValidationError {
            path: "roster".into(),
            message: "roster must have at least one member".into(),
        });
    }
    for (i, m) in def.roster.iter().enumerate() {
        if m.model_ref.is_empty() {
            errs.push(EnsembleValidationError {
                path: format!("roster[{i}].model_ref"),
                message: "model_ref must not be empty".into(),
            });
        }
        if m.alias.is_empty() {
            errs.push(EnsembleValidationError {
                path: format!("roster[{i}].alias"),
                message: "alias must not be empty".into(),
            });
        }
    }

    let valid_combiners = ["linear_opinion_pool", "crps_weighted", "stacking"];
    if !valid_combiners.contains(&def.combiner.as_str()) {
        errs.push(EnsembleValidationError {
            path: "combiner".into(),
            message: format!(
                "unknown combiner '{}'; expected one of: {}",
                def.combiner,
                valid_combiners.join(", ")
            ),
        });
    }

    if def.weight_floor <= 0.0 || def.weight_floor > 1.0 {
        errs.push(EnsembleValidationError {
            path: "weight_floor".into(),
            message: "weight_floor must be in (0, 1]".into(),
        });
    } else if !def.roster.is_empty() && def.weight_floor * def.roster.len() as f64 > 1.0 + 1e-9 {
        errs.push(EnsembleValidationError {
            path: "weight_floor".into(),
            message: format!(
                "weight_floor ({}) × roster size ({}) exceeds 1.0 — infeasible",
                def.weight_floor,
                def.roster.len()
            ),
        });
    }

    if def.temperature <= 0.0 {
        errs.push(EnsembleValidationError {
            path: "temperature".into(),
            message: "temperature must be > 0".into(),
        });
    }

    let valid_cal_methods = ["conformal", "none"];
    if !valid_cal_methods.contains(&def.calibration.method.as_str()) {
        errs.push(EnsembleValidationError {
            path: "calibration.method".into(),
            message: format!(
                "unknown calibration method '{}'; expected one of: {}",
                def.calibration.method,
                valid_cal_methods.join(", ")
            ),
        });
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn two_member_def() -> EnsembleDefinition {
        EnsembleDefinition {
            schema_version: "1.0".into(),
            roster: vec![
                RosterMember {
                    model_ref: "mdl_a".into(),
                    alias: "production".into(),
                },
                RosterMember {
                    model_ref: "mdl_b".into(),
                    alias: "candidate".into(),
                },
            ],
            combiner: "linear_opinion_pool".into(),
            weight_floor: 0.05,
            temperature: 1.0,
            calibration: EnsembleCalibrationSpec::default(),
        }
    }

    #[test]
    fn valid_definition_round_trips() {
        let def = two_member_def();
        assert!(validate_ensemble(&def).is_ok());
        let json = serde_json::to_string(&def).unwrap();
        let def2: EnsembleDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def, def2);
    }

    #[test]
    fn empty_roster_is_rejected() {
        let mut def = two_member_def();
        def.roster.clear();
        let errs = validate_ensemble(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "roster"));
    }

    #[test]
    fn infeasible_weight_floor_is_rejected() {
        let mut def = two_member_def();
        // 2 members × 0.6 floor = 1.2 > 1.0
        def.weight_floor = 0.6;
        let errs = validate_ensemble(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "weight_floor"));
    }

    #[test]
    fn invalid_combiner_is_rejected() {
        let mut def = two_member_def();
        def.combiner = "magic_blend".into();
        let errs = validate_ensemble(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "combiner"));
    }

    #[test]
    fn non_positive_temperature_is_rejected() {
        let mut def = two_member_def();
        def.temperature = 0.0;
        let errs = validate_ensemble(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "temperature"));
    }

    #[test]
    fn all_three_combiners_are_valid() {
        for combiner in ["linear_opinion_pool", "crps_weighted", "stacking"] {
            let mut def = two_member_def();
            def.combiner = combiner.into();
            assert!(
                validate_ensemble(&def).is_ok(),
                "combiner={combiner} rejected"
            );
        }
    }

    #[test]
    fn defaults_produce_valid_definition() {
        let def = EnsembleDefinition {
            schema_version: ENSEMBLE_SCHEMA_VERSION.into(),
            roster: vec![RosterMember {
                model_ref: "mdl_x".into(),
                alias: "production".into(),
            }],
            combiner: default_combiner(),
            weight_floor: default_weight_floor(),
            temperature: default_temperature(),
            calibration: EnsembleCalibrationSpec::default(),
        };
        assert!(validate_ensemble(&def).is_ok());
    }
}
