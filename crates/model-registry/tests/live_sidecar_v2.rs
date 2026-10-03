//! Serving protocol v2 against a running inference sidecar (Set U U-5.2, U-5.7).
//!
//! The unit tests on each side check each side; this checks they agree on the
//! wire: the Rust request reaches the Python service, and its answers come back
//! typed against the contract.
//!
//! ```text
//! cd apps/model-inference && python -m uvicorn app.main:app --port 18082
//! MODEL_INFERENCE_URL=http://127.0.0.1:18082 REGRESSOR_URI=file://… REGRESSOR_SHA=… \
//!   cargo test -j 2 -p model-registry --test live_sidecar_v2 -- --ignored
//! ```

use std::collections::BTreeMap;

use domain::model_def::contract::{ContractOutput, CostClass, IoContract, PitSpec};
use domain::strategy_graph::{Value, ValueType};
use model_registry::provider::outcome_of;
use model_registry::sidecar::{PredictV2Request, SidecarClient, V2Row};
use strategy_engine::ModelOutcome;

fn contract(name: &str) -> IoContract {
    IoContract {
        inputs: vec![],
        outputs: vec![ContractOutput {
            name: name.into(),
            value_type: ValueType::Num,
        }],
        horizon: None,
        pit: PitSpec::default(),
        cost_class: CostClass::Cheap,
        timeout_ms: 1_000,
    }
}

fn row(key: &str, feats: &[(&str, f64)]) -> V2Row {
    V2Row {
        key: key.into(),
        features: feats
            .iter()
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect::<BTreeMap<_, _>>(),
    }
}

#[tokio::test]
#[ignore = "needs a running inference sidecar"]
async fn a_regressor_answers_a_number_and_a_short_row_abstains_by_name() {
    let client = SidecarClient::from_env();
    let resp = client
        .predict_v2(PredictV2Request {
            model_id: "reg".into(),
            version: 1,
            model_kind: "regressor".into(),
            artifact_uri: std::env::var("REGRESSOR_URI").unwrap(),
            artifact_hash: std::env::var("REGRESSOR_SHA").unwrap(),
            rows: vec![
                row("hi", &[("rsi_14", 2.0), ("atr_14", 0.0)]),
                row("lo", &[("rsi_14", -2.0), ("atr_14", 0.0)]),
                row("short", &[("rsi_14", 1.0)]),
            ],
            transform: None,
        })
        .await
        .expect("the sidecar answers");
    let c = contract("value");
    let get = |k: &str| {
        resp.answers
            .iter()
            .find(|a| a.key.as_deref() == Some(k))
            .unwrap()
    };
    let num = |k: &str| match outcome_of(get(k), &c, "reg@1") {
        ModelOutcome::Answered { outputs } => match &outputs["value"] {
            Value::Num(n) => n.get(),
            v => panic!("{v:?}"),
        },
        o => panic!("{k}: {o:?}"),
    };
    assert!(
        num("hi") > num("lo"),
        "a raw regression value, typed as num"
    );
    assert!(
        matches!(outcome_of(get("short"), &c, "reg@1"), ModelOutcome::Abstained { ref detail } if detail.contains("atr_14")),
        "no zero-fill: the missing feature is named"
    );
}

#[tokio::test]
#[ignore = "needs a running inference sidecar"]
async fn a_transform_runs_in_the_sidecar_and_answers_typed() {
    let client = SidecarClient::from_env();
    let resp = client
        .predict_v2(PredictV2Request {
            model_id: "spread".into(),
            version: 1,
            model_kind: "transform".into(),
            artifact_uri: String::new(),
            artifact_hash: String::new(),
            rows: vec![row("k", &[("a", 3.0), ("b", 1.0)])],
            transform: Some(serde_json::json!({
                "language": "python", "entrypoint": "f",
                "source": "def f(x):\n    return {'z': x['a'] - x['b']}\n"
            })),
        })
        .await
        .expect("the sidecar answers");
    let ModelOutcome::Answered { outputs } =
        outcome_of(&resp.answers[0], &contract("z"), "spread@1")
    else {
        panic!("{:?}", resp.answers[0]);
    };
    assert!(matches!(outputs["z"], Value::Num(n) if (n.get() - 2.0).abs() < 1e-12));
}
