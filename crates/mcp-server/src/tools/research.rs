//! Research tools (FEAT-003): Experiments, Studies, the gate funnel, sweeps,
//! diagnostics and the carry-forward read — all thin calls to the platform API.
//!
//! What these tools deliberately cannot do: return a best member, a ranked
//! sample list, or a holdout metric before the vault (INV-2 / INV-3 live on
//! the server, not here).

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::tools::market::urlencode;
use crate::ApiClient;

/// Longest a single `run_sweep` call blocks before returning `timed_out`.
const MAX_WAIT_SECS: u64 = 600;
const POLL_SECS: u64 = 5;

fn missing(field: &str) -> Value {
    json!({ "error": "missing_field", "field": field })
}

fn str_field(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn require_all(params: &Value, keys: &[&str]) -> Result<(), Value> {
    for k in keys {
        if params.get(*k).is_none_or(Value::is_null) {
            return Err(missing(k));
        }
    }
    Ok(())
}

async fn get(api: &ApiClient, path: &str) -> Value {
    match api.get(path).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

async fn post(api: &ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Everything in `params` except `strip` — request bodies are the tool
/// arguments minus the routing ids.
fn body_without(params: &Value, strip: &[&str]) -> Value {
    let mut body = params.clone();
    if let Some(map) = body.as_object_mut() {
        for k in strip {
            map.remove(*k);
        }
    }
    body
}

// ── experiments ───────────────────────────────────────────────────────────────

/// `create_experiment` — a candidate Experiment with a locked holdout and an
/// immutable objective.
pub async fn create_experiment(api: &ApiClient, params: &Value) -> Value {
    if let Err(e) = require_all(
        params,
        &[
            "experiment_id",
            "strategy_family",
            "strategy_type",
            "universe_ref",
            "research_start",
            "research_end",
            "holdout_start",
            "holdout_end",
        ],
    ) {
        return e;
    }
    post(api, "/api/backtest/experiments", params.clone()).await
}

pub async fn list_experiments(api: &ApiClient) -> Value {
    get(api, "/api/backtest/experiments").await
}

pub async fn get_experiment(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    get(
        api,
        &format!("/api/backtest/experiments/{}", urlencode(&id)),
    )
    .await
}

// ── studies ───────────────────────────────────────────────────────────────────

/// `run_study` — any Set J Study kind on an Experiment. Blocks until the
/// Study completes (Runs are real backtests).
pub async fn run_study(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    if let Err(e) = require_all(params, &["study_id", "kind", "vary", "metric", "question"]) {
        return e;
    }
    post(
        api,
        &format!("/api/backtest/experiments/{}/studies", urlencode(&id)),
        body_without(params, &["experiment_id"]),
    )
    .await
}

pub async fn list_studies(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    get(
        api,
        &format!("/api/backtest/experiments/{}/studies", urlencode(&id)),
    )
    .await
}

/// `get_carried_forward` — the parameter set a Study's pre-declared selection
/// rule carried forward (never an argmax).
pub async fn get_carried_forward(api: &ApiClient, params: &Value) -> Value {
    let (Some(id), Some(study)) = (
        str_field(params, "experiment_id"),
        str_field(params, "study_id"),
    ) else {
        return missing("experiment_id/study_id");
    };
    get(
        api,
        &format!(
            "/api/backtest/experiments/{}/studies/{}/carried-forward",
            urlencode(&id),
            urlencode(&study)
        ),
    )
    .await
}

// ── funnel + nulls ────────────────────────────────────────────────────────────

pub async fn get_funnel(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    get(
        api,
        &format!("/api/backtest/experiments/{}/funnel", urlencode(&id)),
    )
    .await
}

/// `advance_gate` — run the next gate (0 integrity → 1 single path → 2
/// robustness → 3 significance). Gate 3 needs a chosen null.
pub async fn advance_gate(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        &format!(
            "/api/backtest/experiments/{}/funnel/advance",
            urlencode(&id)
        ),
        json!({}),
    )
    .await
}

pub async fn get_null_picker(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    get(
        api,
        &format!("/api/backtest/experiments/{}/nulls", urlencode(&id)),
    )
    .await
}

pub async fn choose_null(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    if str_field(params, "kind").is_none() {
        return missing("kind");
    }
    post(
        api,
        &format!("/api/backtest/experiments/{}/nulls", urlencode(&id)),
        body_without(params, &["experiment_id"]),
    )
    .await
}

// ── sweeps ────────────────────────────────────────────────────────────────────

pub async fn start_sweep(api: &ApiClient, params: &Value) -> Value {
    if let Err(e) = require_all(params, &["experiment_id", "question"]) {
        return e;
    }
    post(api, "/api/research/sweeps", params.clone()).await
}

pub async fn get_sweep(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "sweep_id") else {
        return missing("sweep_id");
    };
    get(api, &format!("/api/research/sweeps/{}", urlencode(&id))).await
}

pub async fn cancel_sweep(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "sweep_id") else {
        return missing("sweep_id");
    };
    post(
        api,
        &format!("/api/research/sweeps/{}/cancel", urlencode(&id)),
        json!({}),
    )
    .await
}

fn is_terminal(snapshot: &Value) -> bool {
    matches!(
        snapshot.get("status").and_then(|v| v.as_str()),
        Some("completed" | "failed" | "cancelled")
    )
}

/// `run_sweep` — start a sweep (or resume waiting on `sweep_id`) and block
/// server-side until it finishes or `timeout_seconds` (≤ 600) elapses. On
/// timeout returns `{timed_out: true, sweep_id, ...}` — call again with that
/// `sweep_id`. The internal agent's driver intercepts this tool and waits
/// without the cap at zero token cost.
pub async fn run_sweep(api: &ApiClient, params: &Value) -> Value {
    let sweep_id = match str_field(params, "sweep_id") {
        Some(id) => id,
        None => {
            let started = start_sweep(api, params).await;
            match started.get("sweep_id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => return started, // error surfaced as-is
            }
        }
    };
    let timeout = params
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(MAX_WAIT_SECS)
        .min(MAX_WAIT_SECS);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let snap = get(
            api,
            &format!("/api/research/sweeps/{}", urlencode(&sweep_id)),
        )
        .await;
        if snap.get("error").is_some() || is_terminal(&snap) {
            return snap;
        }
        if Instant::now() >= deadline {
            return json!({
                "timed_out": true,
                "sweep_id": sweep_id,
                "status": snap.get("status").cloned().unwrap_or(Value::Null),
                "done": snap.get("done").cloned().unwrap_or(Value::Null),
                "planned": snap.get("planned").cloned().unwrap_or(Value::Null),
                "hint": "call run_sweep again with this sweep_id to keep waiting",
            });
        }
        tokio::time::sleep(Duration::from_secs(POLL_SECS)).await;
    }
}

// ── diagnostics ───────────────────────────────────────────────────────────────

/// `get_diagnostics` — the diagnostic bundle for a Run reached through one of
/// your Studies: trade shape, monthly slices, worst trades, longest drawdown,
/// exposure, and a compact text summary.
pub async fn get_diagnostics(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    get(
        api,
        &format!("/api/research/diagnostics/{}", urlencode(&run_id)),
    )
    .await
}

/// Declarations for the fifteen tools this module serves (AGENT-006 §2.3).
///
/// Six of these descriptions used to carry a paragraph of how-to-think-about-it
/// prose — `run_sweep` alone was 1,862 bytes, and the whole catalogue is sent on
/// every call. AGENT-005 §5.4 caps a rendered descriptor at 1,200 bytes, so the
/// prose moved verbatim into the authoring guide's `## Tool notes` and is fetched
/// with `get_authoring_guide(section: "tool_notes", tool: "run_sweep")` by the
/// runs that need it. Nothing was deleted; it moved out of the layer every call
/// pays for and into the one a step pays for once.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{ToolSpec, TrialKind};
    use crate::taxonomy::Risk;
    let exp_id = json!({ "type": "string", "description": "Experiment uuid (the `id` field from create_experiment / list_experiments)" });
    let by_exp = |name: &'static str, risk, description, exp: Value| {
        ToolSpec::new(
            name,
            "research",
            risk,
            description,
            json!({ "type": "object", "required": ["experiment_id"], "properties": { "experiment_id": exp } }),
        )
        .internal()
    };
    vec![
        // `backtest_strategy` is namespace `backtest`, not `research`: a step
        // that wants "backtest this" routes to the namespace with the word in
        // it, and the whole point of this tool is that the honest path was too
        // many fields for a 7B to assemble.
        ToolSpec::new(
            "backtest_strategy",
            "backtest",
            Risk::Write,
            "BACKTEST a stored strategy and return the results. One call: creates the Experiment \
             with a locked holdout, runs the sweep (real backtests), returns the report.",
            json!({ "type": "object",
                "required": ["strategy_id"],
                "properties": {
                    "strategy_id": { "type": "string", "description": "The slug you passed to create_strategy." },
                    "instrument_id": { "type": "string", "description": "Default BTC-USD." },
                    "timeframe": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "4h", "1d"], "description": "Default 1h." },
                    "question": { "type": "string", "description": "What you are asking of this sweep, in one line." },
                    "timeout_seconds": { "type": "integer", "description": "Default 600." }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::Search)
        .evidence(&["experiment_id"])
        .max_calls_per_run(10)
        .notes("backtest_strategy"),
        ToolSpec::new(
            "create_experiment",
            "research",
            Risk::Write,
            "Create a candidate Experiment: the unit of honest research. Locks a holdout, \
             declares an immutable objective, starts an irreversible trial counter.",
            json!({ "type": "object",
                "required": ["experiment_id", "strategy_family", "strategy_type", "strategy_ref", "universe_ref", "research_start", "research_end", "holdout_start", "holdout_end", "objective"],
                "properties": {
                    "experiment_id": { "type": "string", "description": "Your slug, e.g. ema_cross_btc_1h" },
                    "strategy_family": { "type": "string", "description": "The idea, version-free" },
                    "strategy_type": { "type": "string", "enum": ["daily_trend", "intraday_momentum", "mean_reversion", "cross_sectional", "event_driven"] },
                    "strategy_ref": { "type": "string", "description": "Stored slug every Run executes" },
                    "universe_ref": { "type": "string", "description": "Instrument id" },
                    "research_start": { "type": "string" },
                    "research_end": { "type": "string", "description": "Exclusive" },
                    "holdout_start": { "type": "string", "description": ">= research_end" },
                    "holdout_end": { "type": "string" },
                    "eval_resolution": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "1d"] },
                    "objective": { "type": "object", "description": "See tool notes" },
                    "target": { "type": "object" }
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .notes("create_experiment"),
        ToolSpec::new(
            "list_experiments",
            "research",
            Risk::Read,
            "Your Experiments, newest first, each with its trial counter, state and gate3 status.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        by_exp(
            "get_experiment",
            Risk::Read,
            "One Experiment header: state, trial counter, objective, strategy_ref, research \
             window. Never holdout metrics.",
            exp_id.clone(),
        )
        .evidence(&["experiment_id"]),
        ToolSpec::new(
            "run_sweep",
            "research",
            Risk::Write,
            "THE inner loop: tune the strategy's declared parameters against the Experiment's \
             objective. Returns carried_forward, the sensitivity surface, the sealed \
             distribution and study_ids. Real backtests: minutes, and counted trials.",
            json!({ "type": "object", "required": ["experiment_id", "question"],
                "properties": {
                    "experiment_id": exp_id.clone(),
                    "sweep_id": { "type": "string", "description": "Resume an existing sweep" },
                    "question": { "type": "string", "description": "What this sweep answers; logged first" },
                    "narrowing": { "type": "object", "description": "Per-parameter tightening only" },
                    "sampler": { "type": "string", "enum": ["tpe", "random"], "description": "Default tpe" },
                    "max_runs": { "type": "integer", "description": "Sampled Runs (default 40)" },
                    "batch_size": { "type": "integer", "description": "Runs per batch (default 8)" },
                    "seed": { "type": "integer" },
                    "base_params": { "type": "object", "description": "Values held fixed" },
                    "objective": { "type": "object", "description": "Only if the Experiment has none" },
                    "timeout_seconds": { "type": "integer", "description": "<= 600" }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::Search)
        .evidence(&["sweep_id"])
        .max_calls_per_run(10)
        .notes("run_sweep"),
        ToolSpec::new(
            "get_sweep",
            "research",
            Risk::Read,
            "Status and progress of a sweep and, when completed, its report.",
            json!({ "type": "object", "required": ["sweep_id"], "properties": { "sweep_id": { "type": "string" } } }),
        )
        .internal()
        .evidence(&["sweep_id"]),
        ToolSpec::new(
            "cancel_sweep",
            "research",
            Risk::Write,
            "Request cancellation of a running sweep (honoured between batches).",
            json!({ "type": "object", "required": ["sweep_id"], "properties": { "sweep_id": { "type": "string" } } }),
        )
        .internal(),
        ToolSpec::new(
            "run_study",
            "research",
            Risk::Write,
            "Run one honest-evaluation Study on the Experiment (counted trials). Returns the \
             sealed distribution and verdict. Prefer run_sweep for parameter tuning.",
            json!({ "type": "object", "required": ["experiment_id", "study_id", "kind", "vary", "metric", "question"],
                "properties": {
                    "experiment_id": exp_id.clone(),
                    "study_id": { "type": "string" },
                    "kind": { "type": "string", "description": "See tool notes for the ten kinds" },
                    "vary": { "type": "object", "description": "Tagged by `vary`; see tool notes" },
                    "metric": { "type": "string", "enum": ["cagr", "total_return", "sharpe", "sortino", "calmar", "detrended_sharpe", "max_drawdown", "profit_factor", "expectancy"] },
                    "question": { "type": "string" },
                    "selection_rule": { "type": "string", "enum": ["none", "median_stable_centroid", "worst_case_robust"] },
                    "null_ref": { "type": "string", "description": "Required for permutation_null" },
                    "base_params": { "type": "object", "description": "Centre the base config on these params" }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::Search)
        .evidence(&["study_id"])
        .max_calls_per_run(10)
        .notes("run_study"),
        by_exp(
            "list_studies",
            Risk::Read,
            "Every Study on an Experiment with its sealed distribution and verdict.",
            exp_id.clone(),
        ),
        ToolSpec::new(
            "get_carried_forward",
            "research",
            Risk::Read,
            "The parameter set a Study's pre-declared selection rule carried forward: the stable \
             centroid or worst-case member, never the best. 404 if it declared no rule.",
            json!({ "type": "object", "required": ["experiment_id", "study_id"], "properties": { "experiment_id": exp_id.clone(), "study_id": { "type": "string" } } }),
        )
        .internal(),
        ToolSpec::new(
            "get_diagnostics",
            "research",
            Risk::Read,
            "Diagnostic bundle for one Run: trade P&L shape, expectancy, streaks, MAE/MFE, \
             monthly slices, worst trades, drawdown episode, exposure and cost drag. Read it \
             before proposing a structural edit.",
            json!({ "type": "object", "required": ["run_id"], "properties": { "run_id": { "type": "string" } } }),
        )
        .internal()
        .evidence(&["run_id"])
        .notes("get_diagnostics"),
        by_exp(
            "get_funnel",
            Risk::Read,
            "The staged-gate ledger for an Experiment: which of Integrity, SinglePath, \
             Robustness, Significance and Vault have passed, with each verdict's null and trial \
             count.",
            exp_id.clone(),
        ),
        // Advancing a gate is a verdict about evidence. It is recorded, ordered
        // and cannot be walked back without the ledger showing it (D-8).
        by_exp(
            "advance_gate",
            Risk::Write,
            "Run the next gate. Gate 2 runs CPCV plus synthetic paths; Gate 3 runs the chosen \
             permutation null with selection-bias correction. Blocks for minutes.",
            exp_id.clone(),
        )
        .spends_trial(TrialKind::Falsification)
        .max_calls_per_run(5)
        .notes("advance_gate"),
        by_exp(
            "get_null_picker",
            Risk::Read,
            "The recommended significance null for this Experiment's strategy type, the full \
             catalog with what each null preserves and destroys, and the logged choice if made.",
            exp_id.clone(),
        ),
        // Choosing a null seals a distribution (INV-2). The choice is final by
        // design, which is what makes it destructive rather than a write.
        ToolSpec::new(
            "choose_null",
            "research",
            Risk::Destructive,
            "Declare the primary significance null. Logged and immutable: there is no second \
             choice.",
            json!({ "type": "object", "required": ["experiment_id", "kind"], "properties": { "experiment_id": exp_id, "kind": { "type": "string", "description": "See tool notes for the seven kinds" }, "override_reason": { "type": "string", "description": "Required when not choosing the recommended null" } } }),
        )
        .internal()
        .notes("choose_null"),
    ]
}

// ── The backtest pipeline ───────────────────────────────────────────────────

/// `backtest_strategy` — experiment, sweep, report. One call.
///
/// The honest path to a backtest on this platform is an Experiment with a locked
/// holdout and an immutable objective, swept by `run_sweep`. That is the right design
/// and it asks the caller for ten fields, two of which are non-overlapping date
/// windows. Measured on the degraded tier: given the step "run a backtest on the
/// strategy", the model reached for `get_backtest` — the one exposed tool with the
/// word in its name — and passed it the id of the TRAINING run, because that was the
/// only uuid it had seen. It did that on every retry until the step budget went.
///
/// It was not confused about backtesting. It was unable to assemble a ten-field
/// request with two date ranges, and nothing in the catalogue offered a smaller move.
/// So this is the same pipeline answer that made `train_model` work: derive the
/// windows, supply a defensible default objective, run the sweep, return the report.
///
/// Defaults are conservative and stated in the reply, so a caller that wanted
/// something else can see exactly what it got instead of discovering it later.
pub async fn backtest_strategy(
    api: &ApiClient,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<crate::ProgressUpdate>>,
) -> Value {
    let strategy_id = params
        .get("strategy_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if strategy_id.is_empty() {
        return json!({
            "error": "missing_field",
            "field": "strategy_id",
            "fix": "the slug you passed to create_strategy"
        });
    }
    let instrument = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("BTC-USD");
    let timeframe = params
        .get("timeframe")
        .and_then(|v| v.as_str())
        .unwrap_or("1h");

    // Windows derived from now, with the holdout as a tail that does not overlap the
    // research period. Explicit dates stay available for a caller that has them.
    let now = chrono::Utc::now();
    let day = chrono::Duration::days(1);
    let holdout_end = now;
    let holdout_start = now - day * 30;
    let research_end = holdout_start;
    let research_start = now - day * 210;
    let iso = |t: chrono::DateTime<chrono::Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let experiment_id = format!("{strategy_id}_exp");
    let objective = params.get("objective").cloned().unwrap_or_else(|| {
        json!({
            "primary": "sortino",
            "constraints": [{ "kind": "min_trades", "value": 20 }],
            "aggregate": "median"
        })
    });

    // ── 1. Experiment (idempotent on the slug) ──────────────────────────────
    let create = json!({
        "experiment_id": experiment_id,
        "strategy_family": strategy_id,
        "strategy_type": "single",
        "strategy_ref": strategy_id,
        "universe_ref": format!("{instrument}:{timeframe}"),
        "research_start": iso(research_start),
        "research_end": iso(research_end),
        "holdout_start": iso(holdout_start),
        "holdout_end": iso(holdout_end),
        "objective": objective,
    });
    let created = create_experiment(api, &create).await;
    // An existing experiment with this slug is not an error — it is a retry.
    let exp_uuid = created
        .get("id")
        .or_else(|| created.get("experiment").and_then(|e| e.get("id")))
        .and_then(Value::as_str)
        .map(str::to_string);

    let exp_uuid = match exp_uuid {
        Some(u) => u,
        None => {
            // Fall back to finding it, so a second call after a partial failure works.
            let listed = list_experiments(api).await;
            let found = listed
                .get("experiments")
                .and_then(Value::as_array)
                .and_then(|arr| {
                    arr.iter()
                        .find(|e| {
                            e.get("experiment_id").and_then(Value::as_str) == Some(&experiment_id)
                        })
                        .and_then(|e| e.get("id").and_then(Value::as_str))
                        .map(str::to_string)
                });
            match found {
                Some(u) => u,
                None => {
                    return json!({
                        "error": "experiment_failed",
                        "detail": "could not create or find the experiment for this strategy",
                        "response": created
                    })
                }
            }
        }
    };

    // ── 2. Sweep — these are real backtests ─────────────────────────────────
    if let Some(tx) = &progress {
        let _ = tx
            .send(crate::ProgressUpdate {
                progress: 20.0,
                message: format!("running backtests for {strategy_id}"),
            })
            .await;
    }
    let sweep = json!({
        "experiment_id": exp_uuid,
        "question": params
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("does this strategy have an edge on the research window"),
        "timeout_seconds": params
            .get("timeout_seconds")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(600)
            .clamp(60, 3600),
    });
    let report = run_sweep(api, &sweep).await;

    json!({
        // `is_none()` would be wrong here for the same reason it was wrong in the
        // bridge: a report carrying `"error": null` is a report with no error.
        "ok": report.get("error").is_none_or(serde_json::Value::is_null),
        "strategy_id": strategy_id,
        "experiment_id": exp_uuid,
        // Stated, not silent: these were derived, and a caller that wanted a
        // different window should be able to see that it did not get one.
        "windows": {
            "research": [iso(research_start), iso(research_end)],
            "holdout_locked": [iso(holdout_start), iso(holdout_end)],
        },
        "objective": objective,
        "report": report,
    })
}
