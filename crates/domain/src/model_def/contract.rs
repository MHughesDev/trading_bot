//! Model IO contracts (FEAT-007 §8.1, ADR-0043).
//!
//! Every model version declares what it takes and what it gives, in the same
//! type system strategies use ([`ValueType`]). The strategy compiler checks every
//! wiring against the contract, so feeding a model the wrong kind of data is a
//! validation error, not a run-time surprise — and a model's answer reaches the
//! strategy in its own shape: a classifier's probabilities as `probs`, a
//! regressor's value as `num`, a forecaster's quantiles as `dist`, an embedding
//! as `vec`. Nothing is squeezed into a forecast any more.
//!
//! Existing models predate contracts; [`derived_contract`] gives each kind the
//! contract its serving path actually honours, so they keep working unchanged.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::kinds::ModelKind;
use super::{ModelDefinition, DEFAULT_QUANTILE_LEVELS};
use crate::strategy_graph::graph::TimeframeKey;
use crate::strategy_graph::value::{Level, ValueType};

/// Where a model's feature inputs come from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureSource {
    /// A registered feature set.
    Set { name: String },
    /// Explicit features in the order the model was trained on — the bundle
    /// header's `feature_order`.
    Named { features: Vec<String> },
}

/// The shape of one model input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum InputShape {
    /// One value of a type — e.g. another model's output wired in.
    Scalar { value_type: ValueType },
    /// One row of features at the decision instant.
    Row { features: FeatureSource },
    /// The last `lookback` rows, oldest first.
    Window {
        features: FeatureSource,
        lookback: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeframe: Option<TimeframeKey>,
    },
    /// One row per universe member (selection strategies).
    CrossSection { features: FeatureSource },
    /// Text — an LLM prompt, rendered from `template` when given.
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
}

/// One named input. (No `deny_unknown_fields`: serde does not support it beside
/// `flatten`, and the flattened shape is what makes the wire form readable.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractInput {
    pub name: String,
    #[serde(flatten)]
    pub shape: InputShape,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractOutput {
    pub name: String,
    pub value_type: ValueType,
}

/// Point-in-time metadata: what a backtest needs to know to call this model
/// honestly (ADR-0043 decision 5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PitSpec {
    /// Trained walk-forward, so each fold can answer for its own test window.
    #[serde(default)]
    pub walk_forward: bool,
    /// Gap between a fold's training end and the earliest instant it may answer
    /// for, e.g. the label horizon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embargo: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    /// In-process or a local sidecar; call every decision.
    #[default]
    Cheap,
    Moderate,
    /// Metered (LLMs): subject to cost caps.
    Expensive,
}

/// What a model takes and gives.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawContract", into = "RawContract")]
pub struct IoContract {
    pub inputs: Vec<ContractInput>,
    pub outputs: Vec<ContractOutput>,
    pub horizon: Option<String>,
    pub pit: PitSpec,
    pub cost_class: CostClass,
    pub timeout_ms: u32,
}

impl IoContract {
    #[must_use]
    pub fn output(&self, name: &str) -> Option<&ContractOutput> {
        self.outputs.iter().find(|o| o.name == name)
    }

    #[must_use]
    pub fn input(&self, name: &str) -> Option<&ContractInput> {
        self.inputs.iter().find(|i| i.name == name)
    }

    /// The model's whole answer as one record type — what a `model` node outputs.
    #[must_use]
    pub fn output_type(&self) -> ValueType {
        ValueType::Record {
            fields: self
                .outputs
                .iter()
                .map(|o| (o.name.clone(), o.value_type.clone()))
                .collect::<BTreeMap<_, _>>(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContract {
    inputs: Vec<ContractInput>,
    outputs: Vec<ContractOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    horizon: Option<String>,
    #[serde(default)]
    pit: PitSpec,
    #[serde(default)]
    cost_class: CostClass,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u32,
}

const fn default_timeout_ms() -> u32 {
    250
}

impl TryFrom<RawContract> for IoContract {
    type Error = ContractError;
    fn try_from(r: RawContract) -> Result<Self, Self::Error> {
        let mut seen = BTreeSet::new();
        for o in &r.outputs {
            if !seen.insert(o.name.as_str()) {
                return Err(ContractError::DuplicateOutput(o.name.clone()));
            }
        }
        if r.outputs.is_empty() {
            return Err(ContractError::NoOutputs);
        }
        let mut seen = BTreeSet::new();
        for i in &r.inputs {
            if !seen.insert(i.name.as_str()) {
                return Err(ContractError::DuplicateInput(i.name.clone()));
            }
        }
        if r.timeout_ms == 0 {
            return Err(ContractError::ZeroTimeout);
        }
        Ok(Self {
            inputs: r.inputs,
            outputs: r.outputs,
            horizon: r.horizon,
            pit: r.pit,
            cost_class: r.cost_class,
            timeout_ms: r.timeout_ms,
        })
    }
}

impl From<IoContract> for RawContract {
    fn from(c: IoContract) -> Self {
        Self {
            inputs: c.inputs,
            outputs: c.outputs,
            horizon: c.horizon,
            pit: c.pit,
            cost_class: c.cost_class,
            timeout_ms: c.timeout_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("two outputs are named '{0}'")]
    DuplicateOutput(String),
    #[error("two inputs are named '{0}'")]
    DuplicateInput(String),
    #[error("a contract must declare at least one output")]
    NoOutputs,
    #[error(
        "timeout_ms must be positive; a model that may take for ever blocks a decision for ever"
    )]
    ZeroTimeout,
}

fn out(name: &str, value_type: ValueType) -> ContractOutput {
    ContractOutput {
        name: name.into(),
        value_type,
    }
}

/// The contract an existing model's serving path actually honours, derived from
/// its kind, its definition and — when the bundle records one — its training
/// feature order.
///
/// `None` for [`ModelKind::Transform`]: a transform is custom code and **must**
/// declare its own contract; there is nothing to derive one from.
#[must_use]
pub fn derived_contract(
    kind: ModelKind,
    def: &ModelDefinition,
    feature_order: Option<&[String]>,
) -> Option<IoContract> {
    let features = match (feature_order, def.feature_set_ref.as_deref()) {
        (Some(names), _) if !names.is_empty() => FeatureSource::Named {
            features: names.to_vec(),
        },
        (_, Some(set)) => FeatureSource::Set { name: set.into() },
        _ => FeatureSource::Set {
            name: "default".into(),
        },
    };
    let row = || ContractInput {
        name: "features".into(),
        shape: InputShape::Row {
            features: features.clone(),
        },
        optional: false,
    };
    let horizon = def.target.as_ref().map(|t| t.horizon.clone());
    let pit = PitSpec {
        walk_forward: def.cv.is_some(),
        embargo: horizon.clone(),
    };
    let cat = |labels: &[&str]| ValueType::Cat {
        labels: labels.iter().map(|s| (*s).to_string()).collect(),
    };

    let (inputs, outputs, cost_class) = match kind {
        ModelKind::Forecaster => {
            let levels = def
                .output
                .as_ref()
                .map_or(DEFAULT_QUANTILE_LEVELS, |o| o.quantile_levels.as_slice())
                .iter()
                .filter_map(|p| Level::new(*p))
                .collect();
            (
                vec![row()],
                vec![
                    out("direction", cat(&["down", "flat", "up"])),
                    out("confidence", ValueType::Num),
                    out("median_return", ValueType::Num),
                    out("sigma", ValueType::Num),
                    out("quantiles", ValueType::Dist { levels }),
                ],
                CostClass::Cheap,
            )
        }
        ModelKind::TradeDecision => (
            vec![row()],
            vec![
                out("action", ValueType::Cat { labels: Vec::new() }),
                out("confidence", ValueType::Num),
            ],
            CostClass::Cheap,
        ),
        ModelKind::RiskSizing => (
            vec![row()],
            vec![out("size_fraction", ValueType::Num)],
            CostClass::Cheap,
        ),
        ModelKind::SignalRanker => (
            vec![row()],
            vec![out("score", ValueType::Num)],
            CostClass::Cheap,
        ),
        ModelKind::Embedding => (
            vec![ContractInput {
                name: "text".into(),
                shape: InputShape::Text { template: None },
                optional: false,
            }],
            vec![out("embedding", ValueType::Vec { dim: None })],
            CostClass::Moderate,
        ),
        ModelKind::ExternalLlmAdapter => (
            vec![ContractInput {
                name: "prompt".into(),
                shape: InputShape::Text { template: None },
                optional: false,
            }],
            vec![out("text", ValueType::Text)],
            CostClass::Expensive,
        ),
        ModelKind::Classifier => (
            vec![row()],
            vec![
                out("class", ValueType::Cat { labels: Vec::new() }),
                out("probs", ValueType::Probs { labels: Vec::new() }),
            ],
            CostClass::Cheap,
        ),
        ModelKind::Regressor => (
            vec![row()],
            vec![out("value", ValueType::Num)],
            CostClass::Cheap,
        ),
        ModelKind::Transform => return None,
    };
    Some(IoContract {
        inputs,
        outputs,
        horizon,
        pit,
        cost_class,
        timeout_ms: if cost_class == CostClass::Expensive {
            10_000
        } else {
            default_timeout_ms()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_def::kinds::Framework;
    use crate::model_def::DEFINITION_VERSION;

    fn def(kind: ModelKind) -> ModelDefinition {
        serde_json::from_value(serde_json::json!({
            "schema_version": DEFINITION_VERSION,
            "model_kind": kind,
            "framework": if matches!(kind, ModelKind::Embedding | ModelKind::ExternalLlmAdapter)
                { Framework::ExternalApi } else { Framework::Lightgbm },
            "asset_class": "crypto_spot_cex",
            "feature_set_ref": "core_v3",
            "target": {"field": "return", "horizon": "4h"}
        }))
        .unwrap()
    }

    const ALL: [ModelKind; 9] = [
        ModelKind::Forecaster,
        ModelKind::SignalRanker,
        ModelKind::TradeDecision,
        ModelKind::RiskSizing,
        ModelKind::Embedding,
        ModelKind::ExternalLlmAdapter,
        ModelKind::Classifier,
        ModelKind::Regressor,
        ModelKind::Transform,
    ];

    #[test]
    fn every_kind_but_transform_has_a_derived_contract() {
        for k in ALL {
            let c = derived_contract(k, &def(k), None);
            assert_eq!(c.is_none(), k == ModelKind::Transform, "{k:?}");
            if let Some(c) = c {
                assert!(!c.outputs.is_empty(), "{k:?}");
                // Round-trips through the validating wire form.
                let back: IoContract =
                    serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
                assert_eq!(back, c, "{k:?}");
            }
        }
    }

    /// FEAT-007 §1 defect 4: every model answered as one `direction` string. The
    /// derived forecaster contract exposes its answer as separate typed fields.
    #[test]
    fn a_forecaster_answers_in_typed_fields_not_one_direction_string() {
        let c = derived_contract(ModelKind::Forecaster, &def(ModelKind::Forecaster), None).unwrap();
        assert!(matches!(
            c.output("direction").unwrap().value_type,
            ValueType::Cat { .. }
        ));
        assert_eq!(c.output("sigma").unwrap().value_type, ValueType::Num);
        let ValueType::Dist { levels } = &c.output("quantiles").unwrap().value_type else {
            panic!("quantiles must be a dist")
        };
        assert_eq!(levels.len(), DEFAULT_QUANTILE_LEVELS.len());
        assert_eq!(c.horizon.as_deref(), Some("4h"));
        assert_eq!(c.pit.embargo.as_deref(), Some("4h"));
    }

    #[test]
    fn a_classifier_answers_with_probabilities() {
        let c = derived_contract(ModelKind::Classifier, &def(ModelKind::Classifier), None).unwrap();
        assert!(matches!(
            c.output("probs").unwrap().value_type,
            ValueType::Probs { .. }
        ));
    }

    #[test]
    fn the_bundle_feature_order_is_the_input_when_known() {
        let order = vec!["rsi_14".to_string(), "atr_14".to_string()];
        let c = derived_contract(
            ModelKind::Regressor,
            &def(ModelKind::Regressor),
            Some(&order),
        )
        .unwrap();
        assert_eq!(
            c.inputs[0].shape,
            InputShape::Row {
                features: FeatureSource::Named { features: order }
            }
        );
    }

    #[test]
    fn llms_are_expensive_and_get_a_longer_timeout() {
        let k = ModelKind::ExternalLlmAdapter;
        let c = derived_contract(k, &def(k), None).unwrap();
        assert_eq!(c.cost_class, CostClass::Expensive);
        assert!(c.timeout_ms > 250);
    }

    #[test]
    fn a_contract_with_two_outputs_of_one_name_is_refused() {
        let err = serde_json::from_value::<IoContract>(serde_json::json!({
            "inputs": [],
            "outputs": [{"name":"x","value_type":{"type":"num"}},
                        {"name":"x","value_type":{"type":"bool"}}]
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("two outputs are named 'x'"), "{err}");
    }

    #[test]
    fn a_contract_with_no_outputs_or_no_timeout_is_refused() {
        assert!(serde_json::from_value::<IoContract>(
            serde_json::json!({"inputs":[],"outputs":[]})
        )
        .is_err());
        assert!(serde_json::from_value::<IoContract>(serde_json::json!({
            "inputs":[], "outputs":[{"name":"x","value_type":{"type":"num"}}], "timeout_ms": 0
        }))
        .is_err());
    }

    #[test]
    fn a_hand_declared_transform_contract_parses() {
        let c: IoContract = serde_json::from_value(serde_json::json!({
            "inputs": [
              {"name":"window","shape":"window","features":{"kind":"set","name":"core_v3"},"lookback":64,"timeframe":"1h"},
              {"name":"regime","shape":"scalar","value_type":{"type":"cat","labels":["calm","trend","stress"]},"optional":true}
            ],
            "outputs": [{"name":"signal","value_type":{"type":"num"}}],
            "pit": {"walk_forward": false},
            "cost_class": "cheap",
            "timeout_ms": 100
        }))
        .unwrap();
        assert_eq!(c.output_type().label(), "record{signal: num}");
        assert!(c.input("regime").unwrap().optional);
    }
}
