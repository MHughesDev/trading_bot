//! Model calls by request and resume (FEAT-007 §8.4, ADR-0043, U-1.6).
//!
//! The engine is synchronous and models are not. So the engine evaluates a tick
//! up to its first model layer and returns [`ModelRequest`]s; the environment
//! resolves them however it resolves models — fold-correct and cached in a
//! backtest, the inference gateway on paper and live — and hands back
//! [`ModelResponse`]s; the engine resumes. The engine never learns which.
//!
//! A request is only ever sent with **every** required input present. A model
//! input the engine cannot assemble — an indicator still warming up, a window not
//! yet full — makes the model node missing *without a call*. Nothing is
//! zero-filled (FEAT-007 §1 defect 4).
//!
//! A response is checked against the model's contract before the strategy sees
//! it: a missing output field or a wrongly-typed one turns the whole answer into
//! `missing(model_abstained)` naming the field, so a misbehaving provider cannot
//! feed a strategy a value of the wrong shape.

use std::collections::BTreeMap;

use domain::model_def::contract::IoContract;
use domain::strategy_graph::{MissingReason, Value, ValueType};
use serde::{Deserialize, Serialize};

/// One assembled model input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelInput {
    /// Feature name → value, at the decision instant.
    Row { values: BTreeMap<String, f64> },
    /// `rows[i][j]` is feature `features[j]` at the i-th bar, oldest first.
    Window {
        features: Vec<String>,
        rows: Vec<Vec<f64>>,
    },
    /// A graph value: another model's output, an indicator, text.
    Value { value: Value },
}

/// A request for one model's answer at one decision instant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// The model node's id — how the response finds its way back.
    pub node: String,
    /// As the strategy wrote it.
    pub reference: String,
    pub model: String,
    /// Pinned. The environment answers with this version or not at all.
    pub version: u32,
    /// The decision instant. A backtest provider uses it to choose the
    /// walk-forward fold that was trained early enough to answer.
    pub at_ns: i64,
    pub inputs: BTreeMap<String, ModelInput>,
}

/// What the environment got back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ModelOutcome {
    /// Named outputs per the contract.
    Answered {
        outputs: BTreeMap<String, Value>,
    },
    /// The model said it had no answer.
    Abstained {
        detail: String,
    },
    TimedOut {
        timeout_ms: u32,
    },
    /// No model could answer — unreachable, or no fold trained early enough.
    Unavailable {
        detail: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    pub node: String,
    pub outcome: ModelOutcome,
}

/// Turn a response into the model node's value: a record per the contract, or
/// missing with the reason.
pub(crate) fn response_value(
    reference: &str,
    contract: &IoContract,
    outcome: &ModelOutcome,
) -> Value {
    match outcome {
        ModelOutcome::Abstained { detail } => Value::Missing(MissingReason::ModelAbstained {
            model: reference.into(),
            detail: detail.clone(),
        }),
        ModelOutcome::TimedOut { timeout_ms } => Value::Missing(MissingReason::ModelTimeout {
            model: reference.into(),
            timeout_ms: *timeout_ms,
        }),
        ModelOutcome::Unavailable { detail } => Value::Missing(MissingReason::ModelUnavailable {
            model: reference.into(),
            detail: detail.clone(),
        }),
        ModelOutcome::Answered { outputs } => {
            let mut record = BTreeMap::new();
            for out in &contract.outputs {
                match outputs.get(&out.name) {
                    // A model may legitimately abstain on one field.
                    Some(v @ Value::Missing(_)) => {
                        record.insert(out.name.clone(), v.clone());
                    }
                    Some(v) if fits(&out.value_type, v) => {
                        record.insert(out.name.clone(), v.clone());
                    }
                    Some(v) => {
                        return Value::Missing(MissingReason::ModelAbstained {
                            model: reference.into(),
                            detail: format!(
                                "output `{}` was a {}, and the contract says {}",
                                out.name,
                                v.kind_name(),
                                out.value_type
                            ),
                        });
                    }
                    None => {
                        return Value::Missing(MissingReason::ModelAbstained {
                            model: reference.into(),
                            detail: format!("the answer has no `{}`", out.name),
                        });
                    }
                }
            }
            Value::Record(record)
        }
    }
}

/// Whether a present value fits a declared type.
pub(crate) fn fits(t: &ValueType, v: &Value) -> bool {
    match (t, v) {
        (ValueType::Num, Value::Num(_))
        | (ValueType::Bool, Value::Bool(_))
        | (ValueType::Text, Value::Text(_))
        | (ValueType::Record { .. }, Value::Record(_)) => true,
        (ValueType::Cat { labels }, Value::Cat(c)) => labels.is_empty() || labels.contains(c),
        (ValueType::Probs { labels }, Value::Probs { labels: got, p }) => {
            got.len() == p.len() && (labels.is_empty() || labels == got)
        }
        (
            ValueType::Dist { levels },
            Value::Dist {
                levels: got,
                values,
            },
        ) => {
            got.len() == values.len()
                && (levels.is_empty() || levels == got)
                && values.windows(2).all(|w| w[0].get() <= w[1].get())
        }
        (ValueType::Vec { dim }, Value::Vec(xs)) => dim.is_none_or(|d| xs.len() == d as usize),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::model_def::contract::{ContractOutput, CostClass, PitSpec};
    use domain::strategy_graph::{Level, Num};

    fn contract() -> IoContract {
        IoContract {
            inputs: Vec::new(),
            outputs: vec![
                ContractOutput {
                    name: "sigma".into(),
                    value_type: ValueType::Num,
                },
                ContractOutput {
                    name: "q".into(),
                    value_type: ValueType::Dist {
                        levels: vec![Level::new(0.1).unwrap(), Level::new(0.9).unwrap()],
                    },
                },
            ],
            horizon: None,
            pit: PitSpec::default(),
            cost_class: CostClass::Cheap,
            timeout_ms: 100,
        }
    }

    fn n(x: f64) -> Num {
        Num::new(x).unwrap()
    }

    #[test]
    fn a_well_formed_answer_becomes_a_record() {
        let outputs = BTreeMap::from([
            ("sigma".to_string(), Value::Num(n(0.02))),
            (
                "q".to_string(),
                Value::Dist {
                    levels: vec![Level::new(0.1).unwrap(), Level::new(0.9).unwrap()],
                    values: vec![n(-0.01), n(0.01)],
                },
            ),
        ]);
        let v = response_value("m@1", &contract(), &ModelOutcome::Answered { outputs });
        assert!(matches!(v, Value::Record(_)));
    }

    #[test]
    fn a_missing_or_mistyped_field_is_an_abstain_naming_it() {
        let only_sigma = BTreeMap::from([("sigma".to_string(), Value::Num(n(0.02)))]);
        let v = response_value(
            "m@1",
            &contract(),
            &ModelOutcome::Answered {
                outputs: only_sigma,
            },
        );
        assert!(
            matches!(&v, Value::Missing(MissingReason::ModelAbstained { detail, .. }) if detail.contains("`q`"))
        );

        let wrong = BTreeMap::from([
            ("sigma".to_string(), Value::Text("high".into())),
            (
                "q".to_string(),
                Value::Missing(MissingReason::NoData { what: "x".into() }),
            ),
        ]);
        let v = response_value(
            "m@1",
            &contract(),
            &ModelOutcome::Answered { outputs: wrong },
        );
        assert!(
            matches!(&v, Value::Missing(MissingReason::ModelAbstained { detail, .. }) if detail.contains("sigma"))
        );
    }

    #[test]
    fn non_monotone_quantiles_do_not_fit_a_dist() {
        let t = ValueType::Dist { levels: Vec::new() };
        let bad = Value::Dist {
            levels: vec![Level::new(0.1).unwrap(), Level::new(0.9).unwrap()],
            values: vec![n(1.0), n(-1.0)],
        };
        assert!(!fits(&t, &bad));
    }

    #[test]
    fn a_timeout_is_a_timeout_and_names_the_model() {
        let v = response_value(
            "m@1",
            &contract(),
            &ModelOutcome::TimedOut { timeout_ms: 100 },
        );
        assert_eq!(
            v,
            Value::Missing(MissingReason::ModelTimeout {
                model: "m@1".into(),
                timeout_ms: 100
            })
        );
    }
}
