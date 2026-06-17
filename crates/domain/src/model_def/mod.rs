//! Model definition format, v1.1 (ADR-0015, ADR-0016, ADR-0017).
//! The validator accepts both v1.0 and v1.1; `migrate_v1_0_to_v1_1` is provided
//! for explicit migration. Adding optional blocks is additive — stored v1.0
//! definitions keep validating and training without modification.

pub mod adapter;
pub mod cv;
pub mod kinds;
pub mod target;
pub mod validate;

use adapter::AdapterSpec;
use cv::WalkForwardSpec;
use kinds::{Framework, ModelKind, Runtime};
use serde::{Deserialize, Serialize};
use target::{InferenceCfg, TargetSpec};

/// Current format version emitted by the builder / validator.
pub const DEFINITION_VERSION: &str = "1.1";

/// Default quantile grid used when migrating a v1.0 definition or when
/// `output` is absent but the model kind is `forecaster`.
pub const DEFAULT_QUANTILE_LEVELS: &[f64] =
    &[0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95];

// ── v1.1 optional blocks ──────────────────────────────────────────────────────

/// Distributional output specification (ADR-0016, I-1.3).
/// Present on `forecaster` models that emit sorted quantiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutputSpec {
    /// Sorted quantile probability levels in (0, 1). Must be strictly increasing.
    pub quantile_levels: Vec<f64>,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self {
            quantile_levels: DEFAULT_QUANTILE_LEVELS.to_vec(),
        }
    }
}

/// Hyperparameter optimisation settings (I-1.9).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HpoSpec {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_trials")]
    pub max_trials: u32,
    /// Optimisation metric: "crps" | "pinball" | "rmse".
    #[serde(default = "default_hpo_metric")]
    pub metric: String,
}

fn default_max_trials() -> u32 {
    40
}
fn default_hpo_metric() -> String {
    "crps".to_string()
}

impl Default for HpoSpec {
    fn default() -> Self {
        Self {
            enabled: false,
            max_trials: default_max_trials(),
            metric: default_hpo_metric(),
        }
    }
}

/// Conformal calibration plan — populated by Phase 4.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSpec {
    /// "conformal" (Phase 4) | "isotonic" | "none"
    #[serde(default = "default_cal_method")]
    pub method: String,
    /// Role used for fitting: "cal" (default) | "val"
    #[serde(default = "default_cal_fit_on")]
    pub fit_on: String,
}

fn default_cal_method() -> String {
    "conformal".to_string()
}
fn default_cal_fit_on() -> String {
    "cal".to_string()
}

impl Default for CalibrationSpec {
    fn default() -> Self {
        Self {
            method: default_cal_method(),
            fit_on: default_cal_fit_on(),
        }
    }
}

// ── ModelDefinition ───────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelDefinition {
    pub schema_version: String,
    pub model_kind: ModelKind,
    pub framework: Framework,
    #[serde(default)]
    pub runtime: Runtime,
    pub asset_class: String,
    #[serde(default)]
    pub target: Option<TargetSpec>,
    #[serde(default)]
    pub feature_set_ref: Option<String>,
    #[serde(default)]
    pub hyperparameters: serde_json::Value,
    #[serde(default)]
    pub label_spec: Option<serde_json::Value>,
    #[serde(default)]
    pub inference: InferenceCfg,
    #[serde(default)]
    pub adapter: Option<AdapterSpec>,
    /// Walk-forward CV plan (ADR-0017). Absent → single expanding-fold back-compat.
    #[serde(default)]
    pub cv: Option<WalkForwardSpec>,
    /// Distributional output config (ADR-0016, v1.1). Absent on v1.0 specs or
    /// non-distributional models; migrated to default quantile grid by `migrate_v1_0_to_v1_1`.
    #[serde(default)]
    pub output: Option<OutputSpec>,
    /// HPO settings (v1.1). Absent → HPO disabled.
    #[serde(default)]
    pub hpo: Option<HpoSpec>,
    /// Calibration plan — populated by Phase 4. Present here for definition round-trips.
    #[serde(default)]
    pub calibration: Option<CalibrationSpec>,
}

// ── Migration ─────────────────────────────────────────────────────────────────

/// Upgrade a v1.0 `ModelDefinition` to v1.1 in-place.
///
/// - Sets `schema_version = "1.1"`.
/// - Fills `output` with the default quantile grid if absent (forecaster kind).
/// - Fills `hpo` with disabled defaults if absent.
/// - Does not touch any field the caller has already set.
pub fn migrate_v1_0_to_v1_1(def: &mut ModelDefinition) {
    def.schema_version = "1.1".to_string();
    if def.output.is_none() && def.model_kind.is_trainable() {
        def.output = Some(OutputSpec::default());
    }
    if def.hpo.is_none() {
        def.hpo = Some(HpoSpec::default());
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use kinds::Framework;
    use target::{TargetField, TargetTransform};

    #[test]
    fn forecaster_v1_0_round_trips() {
        let json = r#"{
            "schema_version": "1.0",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "runtime": "python",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h", "transform": "logret" },
            "feature_set_ref": "fs_core_ohlcv_v3",
            "hyperparameters": { "max_depth": 6, "n_estimators": 400, "learning_rate": 0.05 },
            "label_spec": { "type": "forward_return", "window": "1h", "clip": [-0.2, 0.2] },
            "inference": { "min_confidence": 0.0, "calibrate": true },
            "adapter": null
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.schema_version, "1.0"); // preserves original version
        assert_eq!(def.model_kind, ModelKind::Forecaster);
        assert_eq!(def.framework, Framework::Xgboost);
        let t = def.target.as_ref().unwrap();
        assert_eq!(t.field, TargetField::Return);
        assert_eq!(t.horizon, "1h");
        assert_eq!(t.transform, TargetTransform::Logret);
        // New optional fields absent in v1.0 doc → None
        assert!(def.output.is_none());
        assert!(def.hpo.is_none());
        assert!(def.calibration.is_none());
        let json2 = serde_json::to_string(&def).unwrap();
        let def2: ModelDefinition = serde_json::from_str(&json2).unwrap();
        assert_eq!(def, def2);
    }

    #[test]
    fn forecaster_v1_1_round_trips() {
        let json = r#"{
            "schema_version": "1.1",
            "model_kind": "forecaster",
            "framework": "lightgbm",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" },
            "output": { "quantile_levels": [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95] },
            "hpo": { "enabled": true, "max_trials": 40, "metric": "crps" },
            "calibration": { "method": "conformal", "fit_on": "cal" },
            "cv": {
                "mode": "expanding",
                "folds": 5,
                "train_bars": 4000,
                "cal_bars": 500,
                "test_bars": 500,
                "purge_bars": 8,
                "embargo_bars": 8
            }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.schema_version, DEFINITION_VERSION);
        let out = def.output.as_ref().unwrap();
        assert_eq!(out.quantile_levels.len(), 7);
        assert_eq!(out.quantile_levels[0], 0.05);
        let hpo = def.hpo.as_ref().unwrap();
        assert!(hpo.enabled);
        assert_eq!(hpo.max_trials, 40);
        let cal = def.calibration.as_ref().unwrap();
        assert_eq!(cal.method, "conformal");
        let json2 = serde_json::to_string(&def).unwrap();
        let def2: ModelDefinition = serde_json::from_str(&json2).unwrap();
        assert_eq!(def, def2);
    }

    #[test]
    fn external_llm_adapter_round_trips() {
        let json = r#"{
            "schema_version": "1.0",
            "model_kind": "external_llm_adapter",
            "framework": "external_api",
            "runtime": "python",
            "asset_class": "crypto_spot_cex",
            "adapter": {
                "provider": "ollama",
                "model": "gemma2:9b",
                "endpoint": "http://localhost:11434",
                "default_params": { "temperature": 0.7, "max_tokens": 2048 },
                "cost_per_1k_tokens": "0.0"
            }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(def.model_kind, ModelKind::ExternalLlmAdapter);
        assert!(def.adapter.is_some());
        let json2 = serde_json::to_string(&def).unwrap();
        let def2: ModelDefinition = serde_json::from_str(&json2).unwrap();
        assert_eq!(def, def2);
    }

    #[test]
    fn validate_rejects_wrong_schema_version() {
        let json = r#"{"schema_version":"2.0","model_kind":"forecaster","framework":"xgboost","asset_class":"crypto_spot_cex","target":{"field":"return","horizon":"1h"}}"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let errs = validate::validate(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "schema_version"));
    }

    #[test]
    fn v1_0_definition_validates() {
        let json = r#"{
            "schema_version": "1.0",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        assert!(def.cv.is_none());
        assert!(validate::validate(&def).is_ok());
    }

    #[test]
    fn v1_1_definition_validates() {
        let json = r#"{
            "schema_version": "1.1",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" },
            "output": { "quantile_levels": [0.1, 0.5, 0.9] }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        assert!(validate::validate(&def).is_ok());
    }

    #[test]
    fn definition_with_cv_round_trips_and_validates() {
        let json = r#"{
            "schema_version": "1.0",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" },
            "cv": {
                "mode": "rolling",
                "folds": 5,
                "train_bars": 1000,
                "cal_bars": 200,
                "test_bars": 200,
                "purge_bars": 12,
                "embargo_bars": 12
            }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let cv = def.cv.as_ref().unwrap();
        assert_eq!(cv.mode, cv::WindowMode::Rolling);
        assert_eq!(cv.folds, 5);
        assert!(validate::validate(&def).is_ok());
        let def2: ModelDefinition =
            serde_json::from_str(&serde_json::to_string(&def).unwrap()).unwrap();
        assert_eq!(def, def2);
    }

    #[test]
    fn validate_rejects_cv_with_zero_folds() {
        let json = r#"{
            "schema_version": "1.0",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" },
            "cv": { "folds": 0, "train_bars": 1000, "cal_bars": 200, "test_bars": 200, "embargo_bars": 12 }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let errs = validate::validate(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "cv.folds"));
    }

    #[test]
    fn validate_rejects_adapter_without_adapter_block() {
        let json = r#"{"schema_version":"1.0","model_kind":"external_llm_adapter","framework":"external_api","asset_class":"crypto_spot_cex"}"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let errs = validate::validate(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "adapter"));
    }

    #[test]
    fn validate_rejects_empty_quantile_levels() {
        let json = r#"{
            "schema_version": "1.1",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" },
            "output": { "quantile_levels": [] }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let errs = validate::validate(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "output.quantile_levels"));
    }

    #[test]
    fn validate_rejects_unsorted_quantile_levels() {
        let json = r#"{
            "schema_version": "1.1",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" },
            "output": { "quantile_levels": [0.9, 0.1] }
        }"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let errs = validate::validate(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "output.quantile_levels"));
    }

    #[test]
    fn migrate_v1_0_fills_defaults() {
        let json = r#"{
            "schema_version": "1.0",
            "model_kind": "forecaster",
            "framework": "xgboost",
            "asset_class": "crypto_spot_cex",
            "target": { "field": "return", "horizon": "1h" }
        }"#;
        let mut def: ModelDefinition = serde_json::from_str(json).unwrap();
        migrate_v1_0_to_v1_1(&mut def);
        assert_eq!(def.schema_version, "1.1");
        assert!(def.output.is_some());
        assert_eq!(
            def.output.as_ref().unwrap().quantile_levels,
            DEFAULT_QUANTILE_LEVELS
        );
        assert!(def.hpo.is_some());
        assert!(!def.hpo.as_ref().unwrap().enabled);
        assert!(validate::validate(&def).is_ok());
    }
}
