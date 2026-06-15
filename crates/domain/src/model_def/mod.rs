//! Model definition format, frozen at v1.0 (ADR-0015).
//! Mirrors `crates/domain/src/strategy_def/`.

pub mod adapter;
pub mod kinds;
pub mod target;
pub mod validate;

use serde::{Deserialize, Serialize};
use adapter::AdapterSpec;
use kinds::{Framework, ModelKind, Runtime};
use target::{InferenceCfg, TargetSpec};

pub const DEFINITION_VERSION: &str = "1.0";

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use kinds::Framework;
    use target::{TargetField, TargetSpec, TargetTransform};

    #[test]
    fn forecaster_round_trips() {
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
        assert_eq!(def.schema_version, DEFINITION_VERSION);
        assert_eq!(def.model_kind, ModelKind::Forecaster);
        assert_eq!(def.framework, Framework::Xgboost);
        let t = def.target.as_ref().unwrap();
        assert_eq!(t.field, TargetField::Return);
        assert_eq!(t.horizon, "1h");
        assert_eq!(t.transform, TargetTransform::Logret);
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
        assert!(def.target.is_none());
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
    fn validate_rejects_adapter_without_adapter_block() {
        let json = r#"{"schema_version":"1.0","model_kind":"external_llm_adapter","framework":"external_api","asset_class":"crypto_spot_cex"}"#;
        let def: ModelDefinition = serde_json::from_str(json).unwrap();
        let errs = validate::validate(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "adapter"));
    }
}
