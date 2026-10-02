//! Model providers: how the one strategy engine's model requests are answered
//! (Set U U-5.2, U-5.3, U-5.4; ADR-0043).
//!
//! - [`backtest::SidecarSource`] — the backtest's source behind the
//!   point-in-time rule in `backtest::models::PitModels`.
//! - [`gateway::GatewayModels`] — paper and live: pinned versions, one batched
//!   call per model version per tick, each under its contract's timeout.
//!
//! Both speak serving protocol v2 and type every answer against the version's
//! contract here, before the engine sees it.

pub mod backtest;
pub mod gateway;

use std::collections::BTreeMap;

use domain::model_def::contract::{InputShape, IoContract};
use domain::strategy_graph::{Level, MissingReason, Num, Value, ValueType};
use strategy_engine::{ModelInput, ModelOutcome, ModelRequest};

use crate::sidecar::{PredictV2Request, PredictV2Response, V2Answer};

/// The v2 call, behind a trait so providers can be tested without HTTP.
#[async_trait::async_trait]
pub trait V2Client: Send + Sync {
    async fn predict_v2(&self, req: PredictV2Request) -> anyhow::Result<PredictV2Response>;
}

#[async_trait::async_trait]
impl V2Client for crate::sidecar::SidecarClient {
    async fn predict_v2(&self, req: PredictV2Request) -> anyhow::Result<PredictV2Response> {
        Self::predict_v2(self, req).await
    }
}

/// Answers model requests in a live environment (paper or live). Requests of
/// one tick for one model version arrive together so a provider can answer
/// them in one call; the default answers them one by one.
#[async_trait::async_trait]
pub trait LiveModels: Send + Sync {
    async fn answer(&self, req: ModelRequest) -> ModelOutcome;

    /// Answer several requests to the same pinned version. Returns one outcome
    /// per request, in order.
    async fn answer_batch(&self, reqs: Vec<ModelRequest>) -> Vec<ModelOutcome> {
        let mut out = Vec::with_capacity(reqs.len());
        for r in reqs {
            out.push(self.answer(r).await);
        }
        out
    }
}

/// The feature row a v2 request sends, from the engine's assembled inputs.
///
/// # Errors
/// An input shape the v2 sidecar does not take yet — a window, a scalar, text —
/// named, so the request is unavailable rather than sent half-formed.
pub fn v2_features(
    req: &ModelRequest,
    contract: &IoContract,
) -> Result<BTreeMap<String, f64>, String> {
    let mut row = BTreeMap::new();
    for ci in &contract.inputs {
        match (&ci.shape, req.inputs.get(&ci.name)) {
            (InputShape::Row { .. }, Some(ModelInput::Row { values })) => {
                row.extend(values.iter().map(|(k, v)| (k.clone(), *v)));
            }
            (_, None) if ci.optional => {}
            (_, None) => return Err(format!("input `{}` was not assembled", ci.name)),
            (shape, Some(_)) => {
                return Err(format!(
                    "input `{}` is a {} input, which serving protocol v2 does not take yet",
                    ci.name,
                    match shape {
                        InputShape::Scalar { .. } => "scalar",
                        InputShape::Row { .. } => "row",
                        InputShape::Window { .. } => "window",
                        InputShape::CrossSection { .. } => "cross-section",
                        InputShape::Text { .. } => "text",
                    }
                ))
            }
        }
    }
    Ok(row)
}

fn num(v: &serde_json::Value) -> Option<Num> {
    v.as_f64().and_then(Num::new)
}

/// One plain-JSON output as the value its contract type says.
///
/// # Errors
/// What was wrong with it, naming the field.
pub fn typed_value(name: &str, v: &serde_json::Value, t: &ValueType) -> Result<Value, String> {
    let bad = |what: &str| format!("output `{name}` is not {what}: {v}");
    match t {
        ValueType::Num => num(v).map(Value::Num).ok_or_else(|| bad("a finite number")),
        ValueType::Bool => v.as_bool().map(Value::Bool).ok_or_else(|| bad("a bool")),
        ValueType::Text => v
            .as_str()
            .map(|s| Value::Text(s.into()))
            .ok_or_else(|| bad("text")),
        ValueType::Cat { .. } => v
            .as_str()
            .map(|s| Value::Cat(s.into()))
            .ok_or_else(|| bad("a label")),
        ValueType::Probs { .. } => {
            let labels: Vec<String> = v
                .get("labels")
                .and_then(|l| serde_json::from_value(l.clone()).ok())
                .ok_or_else(|| bad("{labels, p}"))?;
            let p: Vec<Num> = v
                .get("p")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| bad("{labels, p}"))?
                .iter()
                .map(num)
                .collect::<Option<_>>()
                .ok_or_else(|| bad("probabilities"))?;
            Ok(Value::Probs { labels, p })
        }
        ValueType::Dist { .. } => {
            let levels: Vec<Level> = v
                .get("levels")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| bad("{levels, values}"))?
                .iter()
                .map(|x| x.as_f64().and_then(Level::new))
                .collect::<Option<_>>()
                .ok_or_else(|| bad("quantile levels in (0, 1)"))?;
            let values: Vec<Num> = v
                .get("values")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| bad("{levels, values}"))?
                .iter()
                .map(num)
                .collect::<Option<_>>()
                .ok_or_else(|| bad("finite quantile values"))?;
            Ok(Value::Dist { levels, values })
        }
        ValueType::Vec { .. } => v
            .as_array()
            .ok_or_else(|| bad("a vector"))?
            .iter()
            .map(num)
            .collect::<Option<Vec<_>>>()
            .map(Value::Vec)
            .ok_or_else(|| bad("a vector of finite numbers")),
        ValueType::Record { fields } => {
            let obj = v.as_object().ok_or_else(|| bad("a record"))?;
            let mut out = BTreeMap::new();
            for (k, ft) in fields {
                let fv = obj
                    .get(k)
                    .ok_or_else(|| format!("output `{name}` has no field `{k}`"))?;
                out.insert(k.clone(), typed_value(&format!("{name}.{k}"), fv, ft)?);
            }
            Ok(Value::Record(out))
        }
    }
}

/// A v2 answer as the engine takes it, typed against the contract.
///
/// An output the contract names and the model did not give — a point
/// forecaster has no quantiles — is that output `missing`, not the whole answer:
/// a strategy reading `direction` still gets it, one reading `quantiles` holds.
/// An output that is present but malformed turns the whole answer into an
/// abstain naming it.
#[must_use]
pub fn outcome_of(answer: &V2Answer, contract: &IoContract, reference: &str) -> ModelOutcome {
    match answer.status.as_str() {
        "answered" => {
            let mut outputs = BTreeMap::new();
            for out in &contract.outputs {
                let value = match answer.outputs.get(&out.name) {
                    Some(v) => match typed_value(&out.name, v, &out.value_type) {
                        Ok(v) => v,
                        Err(detail) => return ModelOutcome::Abstained { detail },
                    },
                    None => Value::Missing(MissingReason::ModelAbstained {
                        model: reference.into(),
                        detail: format!("the model gave no `{}`", out.name),
                    }),
                };
                outputs.insert(out.name.clone(), value);
            }
            ModelOutcome::Answered { outputs }
        }
        "abstained" => ModelOutcome::Abstained {
            detail: answer.detail.clone().unwrap_or_else(|| "abstained".into()),
        },
        _ => ModelOutcome::Unavailable {
            detail: answer
                .detail
                .clone()
                .unwrap_or_else(|| "unavailable".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::model_def::contract::{ContractOutput, CostClass, PitSpec};

    fn contract(outputs: Vec<(&str, ValueType)>) -> IoContract {
        IoContract {
            inputs: vec![],
            outputs: outputs
                .into_iter()
                .map(|(n, t)| ContractOutput {
                    name: n.into(),
                    value_type: t,
                })
                .collect(),
            horizon: None,
            pit: PitSpec::default(),
            cost_class: CostClass::Cheap,
            timeout_ms: 100,
        }
    }

    fn answered(outputs: serde_json::Value) -> V2Answer {
        V2Answer {
            key: Some("k".into()),
            status: "answered".into(),
            outputs: outputs.as_object().unwrap().clone(),
            detail: None,
        }
    }

    #[test]
    fn every_shape_of_answer_arrives_in_its_own_shape() {
        let c = contract(vec![
            ("class", ValueType::Cat { labels: vec![] }),
            ("probs", ValueType::Probs { labels: vec![] }),
            ("value", ValueType::Num),
            ("embedding", ValueType::Vec { dim: None }),
            ("quantiles", ValueType::Dist { levels: vec![] }),
        ]);
        let a = answered(serde_json::json!({
            "class": "up", "probs": {"labels": ["down", "up"], "p": [0.2, 0.8]}, "value": 0.013,
            "embedding": [0.1, 0.2, 0.3], "quantiles": {"levels": [0.1, 0.5, 0.9], "values": [-0.01, 0.0, 0.01]}
        }));
        let ModelOutcome::Answered { outputs } = outcome_of(&a, &c, "m@1") else {
            panic!()
        };
        assert!(
            matches!(&outputs["probs"], Value::Probs { labels, p } if labels.len() == 2 && p.len() == 2)
        );
        assert!(matches!(&outputs["value"], Value::Num(n) if (n.get() - 0.013).abs() < 1e-12));
        assert!(matches!(&outputs["embedding"], Value::Vec(v) if v.len() == 3));
        assert!(matches!(&outputs["quantiles"], Value::Dist { .. }));
        assert_eq!(outputs["class"], Value::Cat("up".into()));
    }

    #[test]
    fn an_output_the_model_does_not_give_is_missing_and_a_malformed_one_abstains() {
        let c = contract(vec![
            ("direction", ValueType::Cat { labels: vec![] }),
            ("sigma", ValueType::Num),
        ]);
        let ModelOutcome::Answered { outputs } =
            outcome_of(&answered(serde_json::json!({"direction": "up"})), &c, "m@1")
        else {
            panic!()
        };
        assert!(matches!(
            outputs["sigma"],
            Value::Missing(MissingReason::ModelAbstained { .. })
        ));
        let bad = outcome_of(
            &answered(serde_json::json!({"direction": "up", "sigma": "wide"})),
            &c,
            "m@1",
        );
        assert!(matches!(bad, ModelOutcome::Abstained { ref detail } if detail.contains("sigma")));
    }

    #[test]
    fn abstained_and_unavailable_keep_their_reasons() {
        let c = contract(vec![("value", ValueType::Num)]);
        let a = V2Answer {
            key: None,
            status: "abstained".into(),
            outputs: serde_json::Map::new(),
            detail: Some("missing required feature(s): atr_14".into()),
        };
        assert!(
            matches!(outcome_of(&a, &c, "m@1"), ModelOutcome::Abstained { ref detail } if detail.contains("atr_14"))
        );
        let u = V2Answer {
            status: "unavailable".into(),
            ..a
        };
        assert!(matches!(
            outcome_of(&u, &c, "m@1"),
            ModelOutcome::Unavailable { .. }
        ));
    }
}
