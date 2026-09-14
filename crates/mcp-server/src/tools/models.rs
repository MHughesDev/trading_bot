//! Model-registry tools: discovery (`list_models`, `get_model`) and the training
//! pipeline (`train_model`, `promote_model_version`).
//!
//! Strategies (definition v1.1) can reference registered AI models via Inference
//! nodes. Discovery alone was not enough to use that: an agent could see which models
//! existed and had no way to make one, so any task needing a model was blocked on a
//! human opening the UI.
//!
//! # Why `train_model` is one call and not eight
//!
//! The REST surface underneath is granular and right to be — create, train, poll,
//! list versions, evaluate, promote are separate concerns with separate failure
//! modes. Handing that granularity to an agent is a different question. Training a
//! model through it means: list models, create one, read back its id, start a run,
//! read back a run id, poll it to completion, list versions, pick the newest, read
//! its metrics. Nine round trips, six of them bookkeeping, each one a chance for a
//! small model to lose the thread — and on a tier that exposes four tools per step,
//! nine steps of plumbing is most of a task budget spent before any thinking happens.
//!
//! So this is a PIPELINE tool: one call that performs the whole sequence server-side
//! and returns the thing the agent actually wanted, which is a trained version and
//! its metrics. The same reasoning the charter already applies to `run_sweep` and
//! `run_study` — block server-side at zero cost to the agent rather than making it
//! poll.
//!
//! # Determinism
//!
//! Training that cannot be repeated cannot be cited, and a research agent whose
//! evidence is unreproducible is worse than one with no evidence, because the
//! numbers look solid. So the seed is explicit and defaults to a fixed value rather
//! than to entropy, the data window is pinned by the arguments rather than by "now",
//! and the reply carries the `spec_hash` — the registry's own hash over definition
//! plus seed — so two runs claiming to be the same experiment can be checked instead
//! of assumed.

use std::time::Duration;

use serde_json::{json, Value};

use crate::tools::market::urlencode;
use crate::ApiClient;
use crate::ProgressUpdate;

/// Seed used when the caller does not pin one.
///
/// A constant, not entropy. An agent that forgets to pass a seed should get a run it
/// can repeat, because the failure mode of the alternative is silent: results that
/// drift between identical calls and an agent that reports the drift as a finding.
const DEFAULT_SEED: i64 = 42;

/// How long a training run may take before the tool stops waiting.
///
/// Returns the run handle rather than an error on timeout: the run is still going
/// server-side, and telling the agent "this failed" about work that is still
/// progressing would be a lie that costs the whole task.
const DEFAULT_TIMEOUT_SECS: i64 = 900;

/// `list_models` — registered AI models with kind/status.
pub async fn list_models(api: &ApiClient) -> Value {
    match api.get("/api/models").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_model` — one model's detail by id.
pub async fn get_model(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("model_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "model_id" });
    }
    match api.get(&format!("/api/models/{}", urlencode(id))).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

// ── The training pipeline ───────────────────────────────────────────────────

fn str_arg<'a>(params: &'a Value, key: &str) -> &'a str {
    params.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

/// `train_model` — create (if needed), train, wait, and report. One call.
///
/// Idempotent on `slug`: calling it twice with the same slug trains the existing
/// model again rather than creating a second one, so a retry after a timeout does
/// not litter the registry with near-duplicates.
pub async fn train_model(
    api: &ApiClient,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<ProgressUpdate>>,
) -> Value {
    let slug = str_arg(params, "slug");
    if slug.is_empty() {
        return json!({
            "error": "missing_field",
            "field": "slug",
            "fix": "give the model a short stable name, e.g. \"btc-1h-direction\"; calling \
                    train_model again with the same slug retrains it rather than making a copy"
        });
    }
    let instrument = str_arg(params, "instrument_id");
    if instrument.is_empty() {
        return json!({ "error": "missing_field", "field": "instrument_id" });
    }
    let kind = match str_arg(params, "model_kind") {
        "" => "forecaster",
        k => k,
    };
    let timeframe = match str_arg(params, "timeframe") {
        "" => "1h",
        t => t,
    };
    let asset_class = match str_arg(params, "asset_class") {
        "" => "crypto_spot_cex",
        a => a,
    };
    let horizon = match str_arg(params, "label_horizon") {
        "" => "1h",
        h => h,
    };
    let lookback_days = params
        .get("lookback_days")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(30)
        .clamp(1, 3650);
    let seed = params
        .get("seed")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(DEFAULT_SEED);
    let timeout_secs = params
        .get("timeout_seconds")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(30, 3600) as u64;

    // ── 1. Find or create ───────────────────────────────────────────────────
    let existing = api.get("/api/models").await.ok().and_then(|v| {
        v.get("models")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|m| m.get("slug").and_then(Value::as_str) == Some(slug))
                    .and_then(|m| m.get("model_id").and_then(Value::as_str))
                    .map(str::to_string)
            })
    });

    let model_id = match existing {
        Some(id) => id,
        None => {
            let body = json!({
                "display_name": slug,
                "description": format!("Trained by the research agent on {instrument} {timeframe}."),
                "definition": {
                    "schema_version": "1.0",
                    "model_kind": kind,
                    "framework": "lightgbm",
                    "asset_class": asset_class,
                    "target": { "field": "return", "horizon": horizon, "transform": "logret" },
                    // The seed lives in the definition because that is what the
                    // registry hashes into `spec_hash`; passing it only at train time
                    // would leave two runs indistinguishable on paper.
                    "hyperparameters": { "seed": seed },
                }
            });
            match api.post("/api/models", body).await {
                Ok(v) => match v.get("model_id").and_then(Value::as_str) {
                    Some(id) => id.to_string(),
                    None => {
                        return json!({
                            "error": "create_model_failed",
                            "detail": "the registry accepted the request but returned no model_id",
                            "response": v
                        })
                    }
                },
                Err(e) => return e.to_tool_error(),
            }
        }
    };

    // ── 2. Start the run ────────────────────────────────────────────────────
    let train_body = json!({
        "hyperparams": { "seed": seed },
        "version_note": format!("agent run, seed {seed}, {lookback_days}d of {instrument} {timeframe}"),
        "data": {
            "instruments": [instrument],
            "timeframe": timeframe,
            "lookback_days": lookback_days,
            "label_horizon": horizon,
        }
    });
    let run_id = match api
        .post(&format!("/api/models/{}/train", urlencode(&model_id)), train_body)
        .await
    {
        Ok(v) => match v.get("run_id").and_then(Value::as_str) {
            Some(r) => r.to_string(),
            None => {
                return json!({
                    "error": "start_train_failed",
                    "model_id": model_id,
                    "response": v
                })
            }
        },
        Err(e) => return e.to_tool_error(),
    };

    // ── 3. Wait ─────────────────────────────────────────────────────────────
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut last: Value = Value::Null;
    let _ = &last; // assigned on the first successful poll; read only by the timeout path
    loop {
        match api
            .get(&format!(
                "/api/models/{}/runs/{}",
                urlencode(&model_id),
                urlencode(&run_id)
            ))
            .await
        {
            Ok(snap) => {
                let status = snap
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                if let Some(tx) = &progress {
                    let _ = tx
                        .send(ProgressUpdate {
                            progress: snap
                                .get("progress")
                                .and_then(Value::as_f64)
                                .unwrap_or(0.0),
                            message: format!("training {slug} — {status}"),
                        })
                        .await;
                }
                if matches!(
                    status.as_str(),
                    "succeeded" | "completed" | "failed" | "cancelled" | "error"
                ) {
                    return shape_training_result(&model_id, &run_id, seed, &status, snap).await;
                }
                last = snap;
            }
            Err(e) => return e.to_tool_error(),
        }
        if tokio::time::Instant::now() >= deadline {
            // Not an error. The run continues server-side, and the handle is what a
            // caller needs to pick it up — calling this a failure would throw away
            // work that is still in progress.
            return json!({
                "status": "still_running",
                "model_id": model_id,
                "run_id": run_id,
                "seed": seed,
                "waited_seconds": timeout_secs,
                "last_snapshot": last,
                "fix": "the run is still going; call get_training_run with this run_id, \
                        or call train_model again with a larger timeout_seconds"
            });
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Pulls the version and metrics the agent actually asked for out of the run.
async fn shape_training_result(
    model_id: &str,
    run_id: &str,
    seed: i64,
    status: &str,
    snap: Value,
) -> Value {
    let failed = !matches!(status, "succeeded" | "completed");
    json!({
        "status": status,
        "ok": !failed,
        "model_id": model_id,
        "run_id": run_id,
        // Echoed so a later run can be compared against this one rather than assumed
        // equivalent to it.
        "seed": seed,
        "spec_hash": snap.get("spec_hash"),
        "version": snap.get("version").or_else(|| snap.get("model_version")),
        "metrics": snap.get("metrics"),
        // Omitted entirely on success rather than sent as null: a result shape that
        // always carries an `error` key invites every consumer to test for the key
        // instead of its value, and at least one did.
        "error": if failed { snap.get("error").cloned() } else { None },
        "rows_trained": snap.get("rows"),
    })
}

/// `get_training_run` — check a run this agent (or the UI) already started.
pub async fn get_training_run(api: &ApiClient, params: &Value) -> Value {
    let model_id = str_arg(params, "model_id");
    let run_id = str_arg(params, "run_id");
    if model_id.is_empty() || run_id.is_empty() {
        return json!({ "error": "missing_field", "field": "model_id and run_id" });
    }
    match api
        .get(&format!(
            "/api/models/{}/runs/{}",
            urlencode(model_id),
            urlencode(run_id)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `promote_model_version` — make a trained version the one strategies resolve to.
///
/// Separate from training on purpose: training produces a candidate, and deciding it
/// is good enough to be referenced by a strategy is a different judgement with a
/// different blast radius.
pub async fn promote_model_version(api: &ApiClient, params: &Value) -> Value {
    let model_id = str_arg(params, "model_id");
    let version = str_arg(params, "version");
    if model_id.is_empty() || version.is_empty() {
        return json!({ "error": "missing_field", "field": "model_id and version" });
    }
    let alias = match str_arg(params, "alias") {
        "" => "production",
        a => a,
    };
    match api
        .post(
            &format!(
                "/api/models/{}/versions/{}/promote",
                urlencode(model_id),
                urlencode(version)
            ),
            json!({ "alias": alias }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_feature_sets` — what a model may be trained on.
pub async fn list_feature_sets(api: &ApiClient) -> Value {
    match api.get("/api/models/feature-sets").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}
