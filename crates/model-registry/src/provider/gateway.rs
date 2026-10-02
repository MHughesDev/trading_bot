//! The paper/live model provider (Set U U-5.3; ADR-0043 decisions 4 and 5).
//!
//! - **Pinned versions.** A request carries the version its plan was compiled
//!   against; this answers with that version's artifact, read from the registry
//!   snapshot taken when the deployment started. Moving an alias afterwards
//!   changes nothing a running deployment calls.
//! - **Batched.** The live runner hands over every request of one tick to one
//!   version together; they go to the sidecar in **one** v2 call.
//! - Every answer is typed against the contract before the engine sees it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use backtest::models::RegistrySnapshot;
use strategy_engine::{ModelOutcome, ModelRequest};

use super::{outcome_of, v2_features, LiveModels, V2Client};
use crate::sidecar::{PredictV2Request, V2Row};

/// Pinned, batched, typed.
pub struct GatewayModels {
    client: Arc<dyn V2Client>,
    snapshot: Arc<RegistrySnapshot>,
    calls: AtomicU64,
}

impl GatewayModels {
    #[must_use]
    pub fn new(client: Arc<dyn V2Client>, snapshot: Arc<RegistrySnapshot>) -> Self {
        Self {
            client,
            snapshot,
            calls: AtomicU64::new(0),
        }
    }

    /// Sidecar calls made.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl LiveModels for GatewayModels {
    async fn answer(&self, req: ModelRequest) -> ModelOutcome {
        self.answer_batch(vec![req])
            .await
            .pop()
            .unwrap_or(ModelOutcome::Unavailable {
                detail: "no answer".into(),
            })
    }

    async fn answer_batch(&self, reqs: Vec<ModelRequest>) -> Vec<ModelOutcome> {
        let mut out: Vec<Option<ModelOutcome>> = vec![None; reqs.len()];
        // Group by pinned version: one call per version.
        let mut groups: BTreeMap<(String, u32), Vec<usize>> = BTreeMap::new();
        for (i, r) in reqs.iter().enumerate() {
            groups
                .entry((r.model.clone(), r.version))
                .or_default()
                .push(i);
        }
        for ((model, version), idx) in groups {
            let reference = format!("{model}@{version}");
            let unavailable = |detail: String| ModelOutcome::Unavailable { detail };
            let Some(entry) = self.snapshot.pinned(&model, version) else {
                for i in idx {
                    out[i] = Some(unavailable(format!("{reference} is not in the registry")));
                }
                continue;
            };
            let (Some(contract), Some(uri), Some(sha)) =
                (&entry.contract, &entry.artifact_uri, &entry.artifact_sha256)
            else {
                for i in idx {
                    out[i] = Some(unavailable(format!(
                        "{reference} has no contract or no artifact"
                    )));
                }
                continue;
            };
            let mut rows = Vec::new();
            for &i in &idx {
                match v2_features(&reqs[i], contract) {
                    Ok(features) => rows.push(V2Row {
                        key: i.to_string(),
                        features,
                    }),
                    Err(detail) => out[i] = Some(unavailable(detail)),
                }
            }
            if rows.is_empty() {
                continue;
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            let body = PredictV2Request {
                model_id: model.clone(),
                version: i32::try_from(version).unwrap_or(i32::MAX),
                model_kind: entry.kind.clone(),
                artifact_uri: uri.clone(),
                artifact_hash: sha.clone(),
                rows,
            };
            match self.client.predict_v2(body).await {
                Ok(resp) => {
                    for a in &resp.answers {
                        if let Some(i) = a.key.as_deref().and_then(|k| k.parse::<usize>().ok()) {
                            if i < out.len() {
                                out[i] = Some(outcome_of(a, contract, &reference));
                            }
                        }
                    }
                }
                Err(e) => {
                    for &i in &idx {
                        if out[i].is_none() {
                            out[i] = Some(unavailable(format!("sidecar: {e}")));
                        }
                    }
                }
            }
        }
        out.into_iter()
            .map(|o| {
                o.unwrap_or(ModelOutcome::Unavailable {
                    detail: "the sidecar returned no answer for this request".into(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::{PredictV2Response, V2Answer};
    use backtest::models::VersionEntry;
    use domain::model_def::contract::{
        ContractInput, ContractOutput, CostClass, FeatureSource, InputShape, IoContract, PitSpec,
    };
    use domain::strategy_def::params::ParamValues;
    use domain::strategy_graph::{Value, ValueType};
    use std::sync::Mutex;
    use strategy_engine::ModelInput;

    /// Records every request; answers `value = version` so a test can see which
    /// version was called.
    #[derive(Default)]
    struct Fake(Mutex<Vec<PredictV2Request>>);

    #[async_trait::async_trait]
    impl V2Client for Fake {
        async fn predict_v2(&self, req: PredictV2Request) -> anyhow::Result<PredictV2Response> {
            let answers = req
                .rows
                .iter()
                .map(|r| V2Answer {
                    key: Some(r.key.clone()),
                    status: "answered".into(),
                    outputs: serde_json::json!({"value": f64::from(req.version)})
                        .as_object()
                        .unwrap()
                        .clone(),
                    detail: None,
                })
                .collect();
            let resp = PredictV2Response {
                model_id: req.model_id.clone(),
                version: req.version,
                answers,
                latency_ms: 1,
            };
            self.0.lock().unwrap().push(req);
            Ok(resp)
        }
    }

    fn contract() -> IoContract {
        IoContract {
            inputs: vec![ContractInput {
                name: "features".into(),
                shape: InputShape::Row {
                    features: FeatureSource::Named {
                        features: vec!["rsi_14".into()],
                    },
                },
                optional: false,
            }],
            outputs: vec![ContractOutput {
                name: "value".into(),
                value_type: ValueType::Num,
            }],
            horizon: None,
            pit: PitSpec::default(),
            cost_class: CostClass::Cheap,
            timeout_ms: 100,
        }
    }

    fn entry(version: u32) -> VersionEntry {
        VersionEntry {
            model: "reg".into(),
            version,
            kind: "regressor".into(),
            contract: Some(contract()),
            trained_through: None,
            artifact_uri: Some(format!("file:///reg/{version}")),
            artifact_sha256: Some(format!("{version:064}")),
        }
    }

    fn request(node: &str, version: u32) -> ModelRequest {
        ModelRequest {
            node: node.into(),
            reference: "reg@prod".into(),
            model: "reg".into(),
            version,
            at_ns: 0,
            inputs: [(
                "features".to_string(),
                ModelInput::Row {
                    values: [("rsi_14".to_string(), 40.0)].into_iter().collect(),
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    #[tokio::test]
    async fn an_alias_moved_after_deployment_does_not_change_the_version_called() {
        let registry = RegistrySnapshot::new()
            .with_version(entry(3))
            .with_version(entry(4))
            .with_alias("reg", "prod", 3);
        // The deployment compiles against `reg@prod` while it points at 3…
        let graph: domain::strategy_graph::StrategyGraph =
            serde_json::from_value(serde_json::json!({
              "format": "tbot.strategy/3", "kind": "trading", "name": "g",
              "asset_class": "crypto_spot_cex", "timeframe": "1h",
              "nodes": [
                {"id":"m","op":"model","args":{"ref":{"str":"reg@prod"}}},
                {"id":"go","op":"gt","args":{"a":"m.value","b":0}}
              ],
              "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
            }))
            .unwrap();
        let plan = strategy_engine::compile(&graph, &registry, &ParamValues::new()).unwrap();
        let pinned = plan
            .nodes
            .iter()
            .find_map(|n| match &n.kind {
                strategy_engine::plan::NodeKind::Model(c) => Some(c.version),
                _ => None,
            })
            .unwrap();
        assert_eq!(pinned, 3);
        // …then the alias moves to 4. The running deployment's requests carry 3.
        let moved = Arc::new(registry.with_alias("reg", "prod", 4));
        let fake = Arc::new(Fake::default());
        let gw = GatewayModels::new(fake.clone(), moved);
        let out = gw.answer(request("m", pinned)).await;
        let ModelOutcome::Answered { outputs } = out else {
            panic!("{out:?}")
        };
        assert!(matches!(outputs["value"], Value::Num(n) if (n.get() - 3.0).abs() < 1e-12));
        assert_eq!(fake.0.lock().unwrap()[0].version, 3);
        assert_eq!(fake.0.lock().unwrap()[0].artifact_uri, "file:///reg/3");
    }

    #[tokio::test]
    async fn requests_to_one_version_in_one_tick_go_in_one_call() {
        let fake = Arc::new(Fake::default());
        let gw = GatewayModels::new(
            fake.clone(),
            Arc::new(
                RegistrySnapshot::new()
                    .with_version(entry(3))
                    .with_version(entry(4)),
            ),
        );
        let outs = gw
            .answer_batch(vec![
                request("a", 3),
                request("b", 3),
                request("c", 3),
                request("d", 4),
            ])
            .await;
        assert_eq!(outs.len(), 4);
        assert!(outs
            .iter()
            .all(|o| matches!(o, ModelOutcome::Answered { .. })));
        assert_eq!(
            gw.calls(),
            2,
            "one call per pinned version, not one per request"
        );
        let calls = fake.0.lock().unwrap();
        assert_eq!(calls.iter().find(|c| c.version == 3).unwrap().rows.len(), 3);
    }
}
