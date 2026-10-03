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

use super::llm::{cap_breached, estimate_usd, parse_reply, LlmCaps, LlmClient, Spend};
use super::{outcome_of, v2_features, LiveModels, V2Client};
use crate::sidecar::{LlmPredictRequest, PredictV2Request, V2Row};

/// Pinned, batched, typed.
pub struct GatewayModels {
    client: Arc<dyn V2Client>,
    snapshot: Arc<RegistrySnapshot>,
    calls: AtomicU64,
    llm: Option<Arc<dyn LlmClient>>,
    caps: LlmCaps,
    spend: Spend,
}

impl GatewayModels {
    #[must_use]
    pub fn new(client: Arc<dyn V2Client>, snapshot: Arc<RegistrySnapshot>) -> Self {
        Self {
            client,
            snapshot,
            calls: AtomicU64::new(0),
            llm: None,
            caps: LlmCaps::default(),
            spend: Spend::default(),
        }
    }

    /// Answer external LLM adapters through `client`, within `caps` (U-5.6).
    #[must_use]
    pub fn with_llm(mut self, client: Arc<dyn LlmClient>, caps: LlmCaps) -> Self {
        self.llm = Some(client);
        self.caps = caps;
        self
    }

    /// Spent on LLM calls today, in USD.
    #[must_use]
    pub fn spent_today(&self) -> rust_decimal::Decimal {
        self.spend.today()
    }

    /// One LLM request: the prompt it was wired, an instruction to reply with
    /// the contract's fields as JSON, the caps, the call, the strict parse.
    async fn answer_llm(
        &self,
        entry: &backtest::models::VersionEntry,
        req: &ModelRequest,
    ) -> ModelOutcome {
        let reference = format!("{}@{}", entry.model, entry.version);
        let unavailable = |detail: String| ModelOutcome::Unavailable { detail };
        let Some(client) = &self.llm else {
            return unavailable(format!("{reference}: no LLM client in this process"));
        };
        let (Some(contract), Some(adapter)) = (&entry.contract, &entry.adapter) else {
            return unavailable(format!(
                "{reference} has no contract or no adapter settings"
            ));
        };
        let prompt =
            contract
                .inputs
                .iter()
                .find_map(|ci| match (&ci.shape, req.inputs.get(&ci.name)) {
                    (
                        domain::model_def::contract::InputShape::Text { .. },
                        Some(strategy_engine::ModelInput::Value {
                            value: domain::strategy_graph::Value::Text(s),
                        }),
                    ) => Some(s.clone()),
                    _ => None,
                });
        let Some(prompt) = prompt else {
            return unavailable(format!("{reference}: no text prompt was wired"));
        };
        let fields: Vec<String> = contract
            .outputs
            .iter()
            .map(|o| format!("\"{}\": {}", o.name, o.value_type))
            .collect();
        let prompt = format!(
            "{prompt}\n\nReply with only a JSON object with exactly these fields: {{{}}}",
            fields.join(", ")
        );
        let params = adapter
            .get("default_params")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let estimate = estimate_usd(adapter, &params);
        if let Some(why) = cap_breached(&self.caps, &self.spend, estimate) {
            return ModelOutcome::Abstained { detail: why };
        }
        self.calls.fetch_add(1, Ordering::Relaxed);
        let body = LlmPredictRequest {
            model_id: entry.model.clone(),
            version: i32::try_from(entry.version).unwrap_or(i32::MAX),
            adapter: adapter.clone(),
            prompt,
            params,
        };
        match client.predict_llm(body).await {
            Ok(resp) => {
                self.spend.add(resp.cost_usd.parse().unwrap_or(estimate));
                parse_reply(&resp.text, contract)
            }
            Err(e) => unavailable(format!("{reference}: {e}")),
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
            if entry.kind == "external_llm_adapter" {
                for i in idx {
                    out[i] = Some(self.answer_llm(entry, &reqs[i]).await);
                }
                continue;
            }
            let is_transform = entry.kind == "transform" && entry.transform.is_some();
            let (Some(contract), Some((uri, sha))) = (
                &entry.contract,
                match (&entry.artifact_uri, &entry.artifact_sha256) {
                    (Some(u), Some(s)) => Some((u.clone(), s.clone())),
                    // A transform is code, not an artifact (U-5.7).
                    _ if is_transform => Some((String::new(), String::new())),
                    _ => None,
                },
            ) else {
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
                artifact_uri: uri,
                artifact_hash: sha,
                rows,
                transform: entry.transform.clone(),
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
            adapter: None,
            transform: None,
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

    // ── U-5.6: an LLM's answer as a typed value, end to end ──────────────────

    struct FakeLlm(String, Mutex<u32>);

    #[async_trait::async_trait]
    impl LlmClient for FakeLlm {
        async fn predict_llm(
            &self,
            _req: LlmPredictRequest,
        ) -> anyhow::Result<crate::sidecar::LlmPredictResponse> {
            *self.1.lock().unwrap() += 1;
            Ok(crate::sidecar::LlmPredictResponse {
                text: self.0.clone(),
                tokens: 120,
                latency_ms: 5,
                cost_usd: "0.002".into(),
                trace_id: "t".into(),
            })
        }
    }

    fn llm_registry() -> RegistrySnapshot {
        RegistrySnapshot::new().with_version(VersionEntry {
            model: "news".into(),
            version: 1,
            kind: "external_llm_adapter".into(),
            contract: Some(IoContract {
                inputs: vec![ContractInput {
                    name: "prompt".into(),
                    shape: InputShape::Text { template: None },
                    optional: false,
                }],
                outputs: vec![ContractOutput {
                    name: "sentiment".into(),
                    value_type: ValueType::Num,
                }],
                horizon: None,
                pit: PitSpec::default(),
                cost_class: CostClass::Expensive,
                timeout_ms: 2_000,
            }),
            trained_through: None,
            artifact_uri: None,
            artifact_sha256: None,
            adapter: Some(
                serde_json::json!({"provider": "ollama", "model": "gemma2:9b",
                "endpoint": "http://localhost:11434", "cost_per_1k_tokens": "0.01",
                "default_params": {"max_tokens": 500}}),
            ),
            transform: None,
        })
    }

    /// One tick of a strategy whose entry is `sentiment > 0.5`, answered by `gw`.
    async fn decide(gw: &GatewayModels, reg: &RegistrySnapshot) -> strategy_engine::DecisionTrace {
        let graph: domain::strategy_graph::StrategyGraph = serde_json::from_value(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "news_long",
          "asset_class": "crypto_spot_cex", "timeframe": "1h",
          "nodes": [
            {"id":"m","op":"model","args":{"ref":{"str":"news@1"},"prompt":{"str":"Headline: ETF inflows hit a record."}}},
            {"id":"go","op":"gt","args":{"a":"m.sentiment","b":0.5}}
          ],
          "decide": {"rules": [{"when":"go","then":{"kind":"target","size":{"mode":"units","x":1}}}]}
        }))
        .unwrap();
        let plan = strategy_engine::compile(&graph, reg, &ParamValues::new())
            .unwrap_or_else(|d| panic!("{d:#?}"));
        let mut engine = strategy_engine::Engine::new(Arc::new(plan)).unwrap();
        let view = strategy_engine::PortfolioView {
            equity: rust_decimal::Decimal::from(10_000),
            cash: rust_decimal::Decimal::from(10_000),
            mark: Some(rust_decimal::Decimal::from(100)),
            ..Default::default()
        };
        let spec = strategy_engine::InstrumentSpec {
            instrument: "BTC-USD".into(),
            lot_size: rust_decimal::Decimal::new(1, 4),
            tick_size: rust_decimal::Decimal::new(1, 2),
            min_qty: rust_decimal::Decimal::new(1, 4),
            caps: strategy_engine::VenueCaps::all(),
        };
        let bar = strategy_engine::Bar {
            close_ns: 3_600_000_000_000,
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.0,
            volume: 1.0,
        };
        let mut step = engine
            .on_event(
                strategy_engine::Event::Bar {
                    input: 0,
                    bar,
                    warmup: false,
                },
                &view,
                &spec,
            )
            .unwrap();
        while let strategy_engine::Step::NeedModels(reqs) = step {
            let nodes: Vec<String> = reqs.iter().map(|r| r.node.clone()).collect();
            let outcomes = gw.answer_batch(reqs).await;
            let responses = nodes
                .into_iter()
                .zip(outcomes)
                .map(|(node, outcome)| strategy_engine::ModelResponse { node, outcome })
                .collect();
            step = engine.resume(responses).unwrap();
        }
        let strategy_engine::Step::Decided(t) = step else {
            panic!("no decision")
        };
        *t
    }

    #[tokio::test]
    async fn a_sentiment_field_drives_a_threshold_rule_end_to_end() {
        let reg = llm_registry();
        let llm = Arc::new(FakeLlm(r#"{"sentiment": 0.82}"#.into(), Mutex::new(0)));
        let gw = GatewayModels::new(Arc::new(Fake::default()), Arc::new(reg.clone()))
            .with_llm(llm.clone(), LlmCaps::default());
        let t = decide(&gw, &reg).await;
        assert!(
            !t.actions.is_empty(),
            "sentiment 0.82 > 0.5 entered: {:?}",
            t.rules
        );
        assert!(
            gw.spent_today() > rust_decimal::Decimal::ZERO,
            "the call's cost is accounted"
        );

        // A reply that does not match the schema holds the rule — no default.
        let bad = Arc::new(FakeLlm("bullish, probably".into(), Mutex::new(0)));
        let gw = GatewayModels::new(Arc::new(Fake::default()), Arc::new(reg.clone()))
            .with_llm(bad, LlmCaps::default());
        let t = decide(&gw, &reg).await;
        assert!(t.actions.is_empty());
        assert!(
            serde_json::to_string(&t)
                .unwrap()
                .contains("not a JSON object"),
            "the reason is in the trace"
        );
    }

    #[tokio::test]
    async fn past_the_cost_cap_the_model_abstains_and_the_trace_says_why() {
        let reg = llm_registry();
        let llm = Arc::new(FakeLlm(r#"{"sentiment": 0.9}"#.into(), Mutex::new(0)));
        // 500 tokens at $0.01/1k = $0.005 a call, over a $0.001 per-call cap.
        let caps = LlmCaps {
            per_call_usd: rust_decimal::Decimal::new(1, 3),
            per_day_usd: rust_decimal::Decimal::from(1),
        };
        let gw = GatewayModels::new(Arc::new(Fake::default()), Arc::new(reg.clone()))
            .with_llm(llm.clone(), caps);
        let t = decide(&gw, &reg).await;
        assert_eq!(*llm.1.lock().unwrap(), 0, "the call was never made");
        assert!(t.actions.is_empty());
        assert!(
            serde_json::to_string(&t).unwrap().contains("cost cap"),
            "journaled with the cap that stopped it"
        );
    }

    // ── U-5.7: classifier, regressor and transform kinds ────────────────────

    /// A sidecar stand-in answering with fixed outputs, recording requests.
    struct Answers(serde_json::Value, Mutex<Vec<PredictV2Request>>);

    #[async_trait::async_trait]
    impl V2Client for Answers {
        async fn predict_v2(&self, req: PredictV2Request) -> anyhow::Result<PredictV2Response> {
            let answers = req
                .rows
                .iter()
                .map(|r| V2Answer {
                    key: Some(r.key.clone()),
                    status: "answered".into(),
                    outputs: self.0.as_object().unwrap().clone(),
                    detail: None,
                })
                .collect();
            let resp = PredictV2Response {
                model_id: req.model_id.clone(),
                version: req.version,
                answers,
                latency_ms: 1,
            };
            self.1.lock().unwrap().push(req);
            Ok(resp)
        }
    }

    fn kind_entry(
        kind: &str,
        contract: IoContract,
        transform: Option<serde_json::Value>,
    ) -> VersionEntry {
        VersionEntry {
            model: "m".into(),
            version: 1,
            kind: kind.into(),
            contract: Some(contract),
            trained_through: None,
            artifact_uri: transform.is_none().then(|| "file:///m/1".to_string()),
            artifact_sha256: transform.is_none().then(|| "0".repeat(64)),
            adapter: None,
            transform,
        }
    }

    async fn one_decision(
        gw: &GatewayModels,
        reg: &RegistrySnapshot,
        nodes: serde_json::Value,
    ) -> strategy_engine::DecisionTrace {
        let graph: domain::strategy_graph::StrategyGraph = serde_json::from_value(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "k",
          "asset_class": "crypto_spot_cex", "timeframe": "1h",
          "nodes": nodes,
          "decide": {"rules": [{"when":"go","then":{"kind":"target","size":{"mode":"units","x":1}}}]}
        }))
        .unwrap();
        let plan = strategy_engine::compile(&graph, reg, &ParamValues::new())
            .unwrap_or_else(|d| panic!("{d:#?}"));
        let mut engine = strategy_engine::Engine::new(Arc::new(plan)).unwrap();
        let view = strategy_engine::PortfolioView {
            equity: rust_decimal::Decimal::from(10_000),
            cash: rust_decimal::Decimal::from(10_000),
            mark: Some(rust_decimal::Decimal::from(100)),
            ..Default::default()
        };
        let spec = strategy_engine::InstrumentSpec {
            instrument: "BTC-USD".into(),
            lot_size: rust_decimal::Decimal::new(1, 4),
            tick_size: rust_decimal::Decimal::new(1, 2),
            min_qty: rust_decimal::Decimal::new(1, 4),
            caps: strategy_engine::VenueCaps::all(),
        };
        // Enough bars for any indicator input to be present.
        let mut last = None;
        for i in 0..80_i64 {
            let c = 100.0 + (i as f64 / 5.0).sin();
            let bar = strategy_engine::Bar {
                close_ns: (i + 1) * 3_600_000_000_000,
                open: c,
                high: c + 1.0,
                low: c - 1.0,
                close: c,
                volume: 1.0,
            };
            let mut step = engine
                .on_event(
                    strategy_engine::Event::Bar {
                        input: 0,
                        bar,
                        warmup: false,
                    },
                    &view,
                    &spec,
                )
                .unwrap();
            while let strategy_engine::Step::NeedModels(reqs) = step {
                let nodes: Vec<String> = reqs.iter().map(|r| r.node.clone()).collect();
                let outcomes = gw.answer_batch(reqs).await;
                step = engine
                    .resume(
                        nodes
                            .into_iter()
                            .zip(outcomes)
                            .map(|(node, outcome)| strategy_engine::ModelResponse { node, outcome })
                            .collect(),
                    )
                    .unwrap();
            }
            if let strategy_engine::Step::Decided(t) = step {
                last = Some(*t);
            }
        }
        last.expect("decided")
    }

    #[tokio::test]
    async fn a_classifiers_probabilities_drive_a_strategy() {
        let derived = domain::model_def::contract::derived_contract(
            domain::model_def::kinds::ModelKind::Classifier,
            &serde_json::from_value(
                serde_json::json!({"schema_version":"1.1","model_kind":"classifier",
                "framework":"lightgbm","asset_class":"crypto_spot_cex","feature_set_ref":"x"}),
            )
            .unwrap(),
            Some(&["rsi_14".to_string()]),
        )
        .unwrap();
        let reg = RegistrySnapshot::new().with_version(kind_entry("classifier", derived, None));
        let gw = GatewayModels::new(
            Arc::new(Answers(
                serde_json::json!({"class": "up", "probs": {"labels": ["down", "up"], "p": [0.2, 0.8]}}),
                Mutex::default(),
            )),
            Arc::new(reg.clone()),
        );
        let t = one_decision(
            &gw,
            &reg,
            serde_json::json!([
                {"id":"m","op":"model","args":{"ref":{"str":"m@1"}}},
                {"id":"p","op":"prob","args":{"p":"m.probs","label":{"str":"up"}}},
                {"id":"go","op":"gt","args":{"a":"p","b":0.6}}
            ]),
        )
        .await;
        assert!(
            !t.actions.is_empty()
                || t.rules
                    .values()
                    .any(|r| *r == strategy_engine::decide::RuleOutcome::Fired),
            "p(up) = 0.8 > 0.6 fired: {:?}",
            t.rules
        );
    }

    #[tokio::test]
    async fn a_regressors_value_drives_a_strategy() {
        let derived = domain::model_def::contract::derived_contract(
            domain::model_def::kinds::ModelKind::Regressor,
            &serde_json::from_value(
                serde_json::json!({"schema_version":"1.1","model_kind":"regressor",
                "framework":"lightgbm","asset_class":"crypto_spot_cex","feature_set_ref":"x"}),
            )
            .unwrap(),
            Some(&["rsi_14".to_string()]),
        )
        .unwrap();
        let reg = RegistrySnapshot::new().with_version(kind_entry("regressor", derived, None));
        let gw = GatewayModels::new(
            Arc::new(Answers(
                serde_json::json!({"value": 0.012}),
                Mutex::default(),
            )),
            Arc::new(reg.clone()),
        );
        let t = one_decision(
            &gw,
            &reg,
            serde_json::json!([
                {"id":"m","op":"model","args":{"ref":{"str":"m@1"}}},
                {"id":"go","op":"gt","args":{"a":"m.value","b":0.01}}
            ]),
        )
        .await;
        assert_eq!(
            t.rules[&0],
            strategy_engine::decide::RuleOutcome::Fired,
            "0.012 > 0.01"
        );
    }

    #[tokio::test]
    async fn a_transform_is_called_with_its_code_and_no_artifact() {
        let contract = IoContract {
            inputs: vec![ContractInput {
                name: "x".into(),
                shape: InputShape::Scalar {
                    value_type: ValueType::Num,
                },
                optional: false,
            }],
            outputs: vec![ContractOutput {
                name: "z".into(),
                value_type: ValueType::Num,
            }],
            horizon: None,
            pit: PitSpec::default(),
            cost_class: CostClass::Cheap,
            timeout_ms: 100,
        };
        let code = serde_json::json!({"language":"python","entrypoint":"f","source":"def f(i):\n    return {'z': i['x'] * 2}\n"});
        let reg = RegistrySnapshot::new().with_version(kind_entry(
            "transform",
            contract,
            Some(code.clone()),
        ));
        let fake = Arc::new(Answers(serde_json::json!({"z": 4.0}), Mutex::default()));
        let gw = GatewayModels::new(fake.clone(), Arc::new(reg.clone()));
        // Chained: the transform takes a number wired in from another node.
        let t = one_decision(
            &gw,
            &reg,
            serde_json::json!([
                {"id":"two","op":"add","args":{"a":1,"b":1}},
                {"id":"m","op":"model","args":{"ref":{"str":"m@1"},"x":"two"}},
                {"id":"go","op":"gt","args":{"a":"m.z","b":3}}
            ]),
        )
        .await;
        assert_eq!(t.rules[&0], strategy_engine::decide::RuleOutcome::Fired);
        let sent = &fake.1.lock().unwrap()[0];
        assert_eq!(
            sent.transform.as_ref(),
            Some(&code),
            "the code travels with the request"
        );
        assert!(sent.artifact_uri.is_empty(), "and no artifact does");
        assert_eq!(
            sent.rows[0].features.get("x"),
            Some(&2.0),
            "the wired number arrives under its input's name"
        );
    }

    #[tokio::test]
    async fn registering_a_transform_dry_runs_it_against_its_contract() {
        let contract = serde_json::json!({"inputs": [{"name": "x", "shape": "scalar", "value_type": {"type": "num"}}],
            "outputs": [{"name": "z", "value_type": {"type": "num"}}], "timeout_ms": 100});
        let spec = domain::model_def::transform::TransformSpec {
            language: "python".into(),
            entrypoint: "f".into(),
            source: "def f(i):\n    return {'z': 1.0}\n".into(),
        };
        let good = Answers(serde_json::json!({"z": 1.0}), Mutex::default());
        let parsed = crate::contracts::dry_run_transform(&good, "t", &contract, &spec).await;
        assert!(parsed.is_ok(), "{parsed:?}");
        let wrong_shape = Answers(serde_json::json!({"z": "one"}), Mutex::default());
        assert!(
            crate::contracts::dry_run_transform(&wrong_shape, "t", &contract, &spec)
                .await
                .is_err()
        );
        let missing = Answers(serde_json::json!({"y": 1.0}), Mutex::default());
        let err = crate::contracts::dry_run_transform(&missing, "t", &contract, &spec)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("`z`"), "{err}");
        assert!(crate::contracts::dry_run_transform(
            &good,
            "t",
            &serde_json::json!({"outputs": []}),
            &spec
        )
        .await
        .is_err());
    }
}
