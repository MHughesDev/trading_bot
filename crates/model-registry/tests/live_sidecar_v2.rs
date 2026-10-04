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

/// U-5.2's last line: an embedding version returns its `vec`. The Rust client
/// reaches the service's `/v2/embed`, the service the local embedding model,
/// and the vector comes back typed against the embedding contract.
///
/// ```text
/// MODEL_INFERENCE_URL=http://127.0.0.1:18082 EMBED_MODEL=nomic-embed-text \
///   cargo test -j 2 -p model-registry --test live_sidecar_v2 an_embedding -- --ignored
/// ```
#[tokio::test]
#[ignore = "needs MODEL_INFERENCE_URL and a local Ollama serving EMBED_MODEL"]
async fn an_embedding_version_returns_its_vector_typed() {
    use domain::model_def::contract::{ContractInput, InputShape};
    use model_registry::provider::llm::{EmbedRequest, EmbedRow, LlmClient};

    let contract = IoContract {
        inputs: vec![ContractInput {
            name: "text".into(),
            shape: InputShape::Text { template: None },
            optional: false,
        }],
        outputs: vec![ContractOutput {
            name: "embedding".into(),
            value_type: ValueType::Vec { dim: None },
        }],
        horizon: None,
        pit: PitSpec::default(),
        cost_class: CostClass::Moderate,
        timeout_ms: 30_000,
    };
    let texts = [
        "the central bank raised interest rates",
        "the fed hiked rates again",
        "a recipe for sourdough bread",
    ];
    let resp = SidecarClient::from_env()
        .embed(EmbedRequest {
            model_id: "emb".into(),
            version: 1,
            adapter: serde_json::json!({
                "provider": "ollama",
                "model": std::env::var("EMBED_MODEL").expect("EMBED_MODEL"),
            }),
            rows: texts
                .iter()
                .enumerate()
                .map(|(i, t)| EmbedRow {
                    key: i.to_string(),
                    text: (*t).to_string(),
                })
                .collect(),
        })
        .await
        .expect("the service answers");
    let vecs: Vec<Vec<f64>> = resp
        .answers
        .iter()
        .map(|a| match outcome_of(a, &contract, "emb@1") {
            ModelOutcome::Answered { outputs } => match &outputs["embedding"] {
                Value::Vec(v) => v.iter().map(|x| x.get()).collect(),
                other => panic!("not a vec: {other:?}"),
            },
            other => panic!("not answered: {other:?}"),
        })
        .collect();
    assert_eq!(vecs.len(), 3);
    assert!(vecs
        .iter()
        .all(|v| v.len() == vecs[0].len() && v.len() > 100));
    let cos = |a: &[f64], b: &[f64]| {
        let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        dot / (a.iter().map(|x| x * x).sum::<f64>().sqrt()
            * b.iter().map(|y| y * y).sum::<f64>().sqrt())
    };
    println!(
        "dim {}: rates~rates {:.3}, rates~bread {:.3}",
        vecs[0].len(),
        cos(&vecs[0], &vecs[1]),
        cos(&vecs[0], &vecs[2])
    );
    assert!(cos(&vecs[0], &vecs[1]) > cos(&vecs[0], &vecs[2]));
}
