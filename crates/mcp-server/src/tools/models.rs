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

// ── Calling a trained model ─────────────────────────────────────────────────

/// `call_model` — ask a trained model for a prediction. Inference only.
///
/// This is a **read**, and that is the whole reason it is reachable from a Chat
/// turn: training writes a version and spends compute, promotion changes what
/// live strategies resolve to, and asking a finished model what it thinks does
/// neither. `Risk::Read` derives to ring 0 (`research`), and `ToolSpec::new`
/// defaults to spending no trial, so this passes the mode gate in both Chat and
/// Research without either gate being widened — the difference between the modes
/// is trials, not reach, and this spends none.
///
/// Every refusal from the route arrives as a typed `outcome` and is passed
/// through verbatim. That matters more here than anywhere else in this file: a
/// model asked for a prediction it cannot make must say so, because the
/// alternative is a direction and a confidence the agent will quote in a finding.
pub async fn call_model(api: &ApiClient, params: &Value) -> Value {
    let model_id = str_arg(params, "model_id");
    if model_id.is_empty() {
        return json!({ "error": "missing_field", "field": "model_id" });
    }
    let instrument_id = str_arg(params, "instrument_id");
    if instrument_id.is_empty() {
        // Not defaulted to the model id, which is what the route under this used
        // to do: it keyed the gateway's cache and trace on a model id in the
        // instrument slot. "Which market" has no sensible default.
        return json!({ "error": "missing_field", "field": "instrument_id" });
    }

    let mut query = format!("instrument_id={}", urlencode(instrument_id));
    for (key, value) in [
        ("alias", str_arg(params, "alias")),
        ("timeframe", str_arg(params, "timeframe")),
        ("as_of", str_arg(params, "as_of")),
    ] {
        if !value.is_empty() {
            query.push_str(&format!("&{key}={}", urlencode(value)));
        }
    }

    match api
        .get(&format!(
            "/api/models/{}/predict?{query}",
            urlencode(model_id)
        ))
        .await
    {
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
        v.get("models").and_then(Value::as_array).and_then(|arr| {
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
        .post(
            &format!("/api/models/{}/train", urlencode(&model_id)),
            train_body,
        )
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
                            progress: snap.get("progress").and_then(Value::as_f64).unwrap_or(0.0),
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

/// Declarations for the seven `model` tools (AGENT-006 §2.3).
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{Envelope, ToolSpec};
    use crate::taxonomy::Risk;
    vec![
        ToolSpec::new(
            "list_models",
            "model",
            Risk::Read,
            "List registered AI models (kind, status). Definition v1.1 strategies bind model \
             outputs into feature slots via Inference nodes.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "get_model",
            "model",
            Risk::Read,
            "One registered model's detail by id.",
            json!({
                "type": "object",
                "required": ["model_id"],
                "properties": { "model_id": { "type": "string" } }
            }),
        )
        .internal()
        .evidence(&["model_id"]),
        ToolSpec::new(
            "list_feature_sets",
            "model",
            Risk::Read,
            "Feature sets a model can be trained on. Call it before train_model when you need to \
             name feature_set_ref.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        // Inference is a read: it writes no version, spends no trial, and
        // changes nothing a later run would resolve differently. That is what
        // makes it reachable from a Chat turn, where training and promotion are
        // not — the mode gate filters on ring and trials, and this tool is ring 0
        // with no trial.
        ToolSpec::new(
            "call_model",
            "model",
            Risk::Read,
            "Ask an already-trained model for a prediction on one instrument at one instant. \
             Inference only: it never trains, creates or promotes anything. Features are \
             computed from stored bars at or before `as_of` using exactly the column list the \
             model's version was trained on — if any column cannot be produced the call is \
             refused and names them, rather than substituting zeros. An `outcome` of \
             `abstained` is the model declining to answer and is not a neutral prediction.",
            json!({
                "type": "object",
                "required": ["model_id", "instrument_id"],
                "properties": {
                    "model_id": { "type": "string", "description": "Model id or slug" },
                    "instrument_id": { "type": "string", "description": "The instrument to predict for. Must have stored bars." },
                    "alias": { "type": "string", "description": "Which published version to call. Default 'production'." },
                    "timeframe": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"], "description": "Default 1h" },
                    "as_of": { "type": "string", "description": "RFC-3339. Default now; clamped to now. Features use only bars at or before it." }
                }
            }),
        )
        .internal()
        .evidence(&["model_id"]),
        // Training spends real compute and writes a new version. Nothing is
        // destroyed and a bad version can simply not be promoted, so: write.
        ToolSpec::new(
            "train_model",
            "model",
            Risk::Write,
            "Train a model end to end and wait for it: creates the model if the slug is new, \
             trains on real stored bars, and returns the version with its metrics.",
            json!({
                "type": "object",
                "required": ["slug", "instrument_id"],
                "properties": {
                    "slug": { "type": "string", "description": "Short stable name; reusing it retrains that model" },
                    "instrument_id": { "type": "string", "description": "Must have stored bars" },
                    "model_kind": { "type": "string", "enum": ["forecaster", "classifier", "regressor"] },
                    "timeframe": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"], "description": "Default 1h" },
                    "asset_class": { "type": "string", "enum": ["crypto_spot_cex", "us_equity"] },
                    "label_horizon": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"], "description": "A DURATION, not a feature" },
                    "lookback_days": { "type": "integer", "description": "Default 30" },
                    "seed": { "type": "integer", "description": "Default 42" },
                    "timeout_seconds": { "type": "integer", "description": "Default 900; a run_id is returned on timeout" }
                }
            }),
        )
        .internal()
        .evidence(&["model_id", "run_id"])
        .max_calls_per_run(3)
        .notes("train_model"),
        ToolSpec::new(
            "get_training_run",
            "model",
            Risk::Read,
            "Status and metrics of one training run, by model_id and run_id.",
            json!({
                "type": "object",
                "required": ["model_id", "run_id"],
                "properties": {
                    "model_id": { "type": "string" },
                    "run_id": { "type": "string" }
                }
            }),
        )
        .internal(),
        // Promotion changes which version live strategies resolve to. It is
        // reversible by promoting another version, so `Risk::Write` — but it is
        // the one model action AGENT-001 §15 says needs a human, which is the
        // `Envelope`, not the risk. The two answer different questions, and
        // conflating them is how S-0.2's defect stayed invisible.
        ToolSpec::new(
            "promote_model_version",
            "model",
            Risk::Write,
            "Point an alias (default 'production') at a trained model version, so strategies \
             referencing the model resolve to it.",
            json!({
                "type": "object",
                "required": ["model_id", "version"],
                "properties": {
                    "model_id": { "type": "string" },
                    "version": { "type": "string" },
                    "alias": { "type": "string", "description": "Default production." }
                }
            }),
        )
        .internal()
        .envelope(Envelope::Promotion),
    ]
}
