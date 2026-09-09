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
    get(api, &format!("/api/backtest/experiments/{}", urlencode(&id))).await
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
    get(api, &format!("/api/backtest/experiments/{}/studies", urlencode(&id))).await
}

/// `get_carried_forward` — the parameter set a Study's pre-declared selection
/// rule carried forward (never an argmax).
pub async fn get_carried_forward(api: &ApiClient, params: &Value) -> Value {
    let (Some(id), Some(study)) = (str_field(params, "experiment_id"), str_field(params, "study_id"))
    else {
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
    get(api, &format!("/api/backtest/experiments/{}/funnel", urlencode(&id))).await
}

/// `advance_gate` — run the next gate (0 integrity → 1 single path → 2
/// robustness → 3 significance). Gate 3 needs a chosen null.
pub async fn advance_gate(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        &format!("/api/backtest/experiments/{}/funnel/advance", urlencode(&id)),
        json!({}),
    )
    .await
}

pub async fn get_null_picker(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    get(api, &format!("/api/backtest/experiments/{}/nulls", urlencode(&id))).await
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
    post(api, &format!("/api/research/sweeps/{}/cancel", urlencode(&id)), json!({})).await
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
        let snap = get(api, &format!("/api/research/sweeps/{}", urlencode(&sweep_id))).await;
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
    get(api, &format!("/api/research/diagnostics/{}", urlencode(&run_id))).await
}

/// Tool definitions for this module (appended to the server's list).
#[must_use]
pub fn definitions() -> Vec<Value> {
    let exp_id = json!({ "type": "string", "description": "Experiment uuid (the `id` field from create_experiment / list_experiments)" });
    vec![
        json!({
            "name": "create_experiment",
            "description": "Create a candidate Experiment: the unit of honest research. Locks a holdout tail you cannot read until the vault; declares an immutable objective; every Study you run on it increments an irreversible trial counter that deflates significance. Research window must not overlap the holdout. Returns the experiment (use its `id` everywhere else).",
            "inputSchema": { "type": "object",
                "required": ["experiment_id", "strategy_family", "strategy_type", "strategy_ref", "universe_ref", "research_start", "research_end", "holdout_start", "holdout_end", "objective"],
                "properties": {
                    "experiment_id": { "type": "string", "description": "Your slug for this investigation, e.g. ema_cross_btc_1h" },
                    "strategy_family": { "type": "string", "description": "The idea being investigated (version-agnostic), e.g. ema_cross" },
                    "strategy_type": { "type": "string", "enum": ["daily_trend", "intraday_momentum", "mean_reversion", "cross_sectional", "event_driven"], "description": "Seeds the recommended significance null" },
                    "strategy_ref": { "type": "string", "description": "Stored strategy slug (from create_strategy) every Run executes. Must declare a `parameters` block to be sweepable." },
                    "universe_ref": { "type": "string", "description": "Instrument id, e.g. BTC-USD" },
                    "research_start": { "type": "string", "description": "RFC3339" },
                    "research_end": { "type": "string", "description": "RFC3339 — research slice end (exclusive)" },
                    "holdout_start": { "type": "string", "description": "RFC3339 — must be >= research_end" },
                    "holdout_end": { "type": "string", "description": "RFC3339" },
                    "eval_resolution": { "type": "string", "enum": ["1m", "5m", "15m", "1h", "1d"], "description": "Bar timeframe Runs execute on (default 1d). Start on 1h." },
                    "objective": { "type": "object", "description": "{primary: sortino|calmar|profit_factor|expectancy|detrended_sharpe|sharpe, constraints: [{kind: min_trades, value: 50}, {kind: max_drawdown_lte, value: 0.15}, ...], aggregate: median|worst5_pct}. Win rate is never a primary." }
                }
            }
        }),
        json!({
            "name": "list_experiments",
            "description": "Your Experiments, newest first, each with its trial counter, state and gate3 status.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "get_experiment",
            "description": "One Experiment header: state, trial counter, objective, strategy_ref, research window. Never holdout metrics.",
            "inputSchema": { "type": "object", "required": ["experiment_id"], "properties": { "experiment_id": exp_id } }
        }),
        json!({
            "name": "run_sweep",
            "description": "THE inner loop. Tune the strategy's declared `parameters` against the Experiment's objective: a sampler (TPE by default) proposes batches, each batch is a sealed ParameterSweep Study (counted trials), then a neighbourhood Study around the promising region yields the ONLY carry-forward — its median-stable-centroid member, never the best sample. Returns a report with: `carried_forward` (params to build on), `surface.text` (per-parameter plateau/cliff/sensitivity — read this; plateaus are robust, spikes are fragile), `distribution` (sealed), `violation_counts` (why samples were rejected, e.g. too few trades), `study_ids`. Blocks up to timeout_seconds; on `timed_out` call again with the returned sweep_id. Runs are real backtests: 40 runs on 1h bars takes minutes.",
            "inputSchema": { "type": "object", "required": ["experiment_id", "question"],
                "properties": {
                    "experiment_id": exp_id,
                    "sweep_id": { "type": "string", "description": "Resume waiting on an existing sweep instead of starting one" },
                    "question": { "type": "string", "description": "What this sweep answers — logged before running" },
                    "narrowing": { "type": "object", "description": "Optional per-parameter tightening: {fast: {min: 8, max: 30}, exit: {choices: ['trail']}}. Can only narrow the declaration." },
                    "sampler": { "type": "string", "enum": ["tpe", "random"], "description": "Default tpe" },
                    "max_runs": { "type": "integer", "description": "Sampled Runs (default 40; the neighbourhood cube is extra)" },
                    "batch_size": { "type": "integer", "description": "Runs per sealed batch Study (default 8)" },
                    "seed": { "type": "integer" },
                    "base_params": { "type": "object", "description": "Values held fixed for parameters you are not sweeping (e.g. a prior carried_forward)" },
                    "objective": { "type": "object", "description": "Override only if the Experiment has none" },
                    "timeout_seconds": { "type": "integer", "description": "<= 600" }
                }
            }
        }),
        json!({
            "name": "get_sweep",
            "description": "Status/progress of a sweep and, when completed, its report.",
            "inputSchema": { "type": "object", "required": ["sweep_id"], "properties": { "sweep_id": { "type": "string" } } }
        }),
        json!({
            "name": "cancel_sweep",
            "description": "Request cancellation of a running sweep (honoured between batches).",
            "inputSchema": { "type": "object", "required": ["sweep_id"], "properties": { "sweep_id": { "type": "string" } } }
        }),
        json!({
            "name": "run_study",
            "description": "Run one honest-evaluation Study on the Experiment (counted trials). Kinds: parameter_sweep, neighborhood, walk_forward, cpcv, nested_cv, permutation_null, synthetic_paths, cost_sweep, trade_monte_carlo, regime_conditional. Returns the SEALED distribution (median, IQR, worst-5%, spread) and verdict; member ids are provenance only. Use `selection_rule: median_stable_centroid` to carry a config forward (then get_carried_forward). Prefer run_sweep for parameter tuning.",
            "inputSchema": { "type": "object", "required": ["experiment_id", "study_id", "kind", "vary", "metric", "question"],
                "properties": {
                    "experiment_id": exp_id,
                    "study_id": { "type": "string" },
                    "kind": { "type": "string" },
                    "vary": { "type": "object", "description": "Tagged by `vary`: {vary:'params', grid:[{fast:12},{fast:20}]} | {vary:'neighborhood', param:'fast', center:20, step:2, k:3} | {vary:'data_windows', windows:[[start,end],...]} | {vary:'cpcv_groups', n_groups:6, k_test:2} | {vary:'seeds', n:200} | {vary:'cost_ladder', cost_model_refs:[...]} | {vary:'trade_resamples', n:500, block:5} | {vary:'regimes', windows:[[start,end,label],...]}" },
                    "metric": { "type": "string", "enum": ["cagr", "total_return", "sharpe", "sortino", "calmar", "detrended_sharpe", "max_drawdown", "profit_factor", "expectancy"] },
                    "question": { "type": "string" },
                    "selection_rule": { "type": "string", "enum": ["none", "median_stable_centroid", "worst_case_robust"] },
                    "null_ref": { "type": "string", "description": "Required for permutation_null" },
                    "base_params": { "type": "object", "description": "Centre the base config on these params (e.g. a carried_forward set)" }
                }
            }
        }),
        json!({
            "name": "list_studies",
            "description": "Every Study on an Experiment with its sealed distribution and verdict.",
            "inputSchema": { "type": "object", "required": ["experiment_id"], "properties": { "experiment_id": exp_id } }
        }),
        json!({
            "name": "get_carried_forward",
            "description": "The parameter set a Study's pre-declared selection rule carried forward (the stable centroid / worst-case member — never the best). 404 if the Study declared no rule.",
            "inputSchema": { "type": "object", "required": ["experiment_id", "study_id"], "properties": { "experiment_id": exp_id, "study_id": { "type": "string" } } }
        }),
        json!({
            "name": "get_diagnostics",
            "description": "Diagnostic bundle for a Run you reached through one of your Studies (member ids from list_studies / a sweep's study): trade P&L shape, win/loss averages, expectancy, longest losing streak, hold time, MAE/MFE, by-month slices, the ten worst trades, the longest drawdown episode, exposure and cost drag, plus a compact `text`. This is what tells you WHAT to change — read it before proposing a structural edit.",
            "inputSchema": { "type": "object", "required": ["run_id"], "properties": { "run_id": { "type": "string" } } }
        }),
        json!({
            "name": "get_funnel",
            "description": "The staged-gate ledger for an Experiment: which of Integrity → SinglePath → Robustness → Significance → Vault have passed, with each verdict's null and trial count.",
            "inputSchema": { "type": "object", "required": ["experiment_id"], "properties": { "experiment_id": exp_id } }
        }),
        json!({
            "name": "advance_gate",
            "description": "Run the next gate. Gate 2 runs CPCV + synthetic paths (worst-5% must survive); Gate 3 runs the chosen permutation null with selection-bias correction from the live trial counter and requires deflated Sharpe >= 0.95 and PBO <= 0.5. Blocks for minutes. Choose a null (choose_null) before Gate 3.",
            "inputSchema": { "type": "object", "required": ["experiment_id"], "properties": { "experiment_id": exp_id } }
        }),
        json!({
            "name": "get_null_picker",
            "description": "The recommended significance null for this Experiment's strategy type, the full catalog with what each null preserves/destroys, and the logged choice if made.",
            "inputSchema": { "type": "object", "required": ["experiment_id"], "properties": { "experiment_id": exp_id } }
        }),
        json!({
            "name": "choose_null",
            "description": "Declare the primary significance null (logged; immutable). Kinds: signal_return_decouple, block_permutation, stationary_bootstrap, bar_permutation, synthetic_garch, regime_block, random_entry_matched.",
            "inputSchema": { "type": "object", "required": ["experiment_id", "kind"], "properties": { "experiment_id": exp_id, "kind": { "type": "string" }, "override_reason": { "type": "string", "description": "Required when not choosing the recommended null" } } }
        }),
    ]
}
