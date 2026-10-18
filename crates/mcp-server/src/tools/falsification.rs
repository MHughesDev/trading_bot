//! Falsification tools: the agent's primary output surface
//! (EXTERNAL-REVIEW-2026-09 §4; ADR-P6-05, ADR-P6-06, ADR-P6-03).
//!
//! Four tools, and the ordering between two of them is the whole design.
//!
//! - `report_evaluator_defect` — the platform itself is wrong. Scores **above**
//!   a promotion. Without that ordering an agent that discovers the simulator
//!   lets it buy the intrabar low will exploit it silently, because reporting it
//!   would destroy its best result.
//! - `report_kill` — a candidate is refuted. Dense reward: most strategies are
//!   noise, so this fires constantly and is usually right.
//! - `check_tradeable_frontier` — is this horizon searchable at all, before a
//!   trial is spent finding out.
//! - `run_oracle_test` — is the cost model itself sane.
//!
//! Every description here tells the model what does **not** count. A kill is a
//! reproducible defect; "the Sharpe was low" is a result, and the server refuses
//! it at the constructor rather than scoring it zero after the fact.

use serde_json::{json, Value};

use crate::ApiClient;

fn missing(field: &str) -> Value {
    crate::tool_error::missing_field(field)
}

fn str_field(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn steps_field(params: &Value) -> Vec<String> {
    params
        .get("steps")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Shared pre-flight on the reproduction, so the agent gets the *reason* rather
/// than a 422 from the database.
///
/// The server checks again — this is a better error message, not a security
/// boundary, and a check that only exists here is a check an HTTP client skips.
fn check_reproduction(params: &Value) -> Result<(), Value> {
    for k in ["artifact_ref", "expected", "observed"] {
        if str_field(params, k).is_none() {
            return Err(missing(k));
        }
    }
    if params.get("seed").and_then(Value::as_u64).is_none() {
        return Err(crate::tool_error::missing_field_because(
            "seed",
            "a replay that cannot be pinned to a seed is an anecdote",
        ));
    }
    if steps_field(params).is_empty() {
        return Err(crate::tool_error::refuse(
            "no_steps",
            "the report lists no steps, so a human cannot re-run it in ten minutes.",
            "add `steps`: the ordered actions that reproduce the defect",
        ));
    }
    let expected = str_field(params, "expected").unwrap_or_default();
    let observed = str_field(params, "observed").unwrap_or_default();
    if expected == observed {
        return Err(crate::tool_error::refuse(
            "not_falsifiable",
            "`expected` and `observed` are the same, so nothing was refuted. A low score is not a \
             kill: 'Sharpe is only 0.4' is a result, not a defect.",
            "rewrite `expected` as what you predicted would happen and `observed` as what happened instead",
        ));
    }
    Ok(())
}

async fn post(api: &ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `report_kill` — refute a candidate.
pub async fn report_kill(api: &ApiClient, params: &Value) -> Value {
    if let Err(e) = check_reproduction(params) {
        return e;
    }
    let Some(trial_id) = str_field(params, "target_trial_id") else {
        return missing("target_trial_id");
    };
    let Some(defect_class) = str_field(params, "defect_class") else {
        return missing("defect_class");
    };

    post(
        api,
        "/api/falsification/kills",
        json!({
            "target_trial_id": trial_id,
            "defect_class": defect_class,
            "artifact_ref": str_field(params, "artifact_ref"),
            "seed": params.get("seed").and_then(Value::as_u64),
            "expected": str_field(params, "expected"),
            "observed": str_field(params, "observed"),
            "steps": steps_field(params),
        }),
    )
    .await
}

/// `report_evaluator_defect` — the platform is wrong.
pub async fn report_evaluator_defect(api: &ApiClient, params: &Value) -> Value {
    if let Err(e) = check_reproduction(params) {
        return e;
    }
    let Some(component) = str_field(params, "component") else {
        return missing("component");
    };
    let Some(guarantee) = str_field(params, "claimed_guarantee") else {
        return crate::tool_error::missing_field_with(
            "claimed_guarantee",
            "add `claimed_guarantee`: quote what this component claims to guarantee, from the spec \
             or its own docs; without the claim there is only a behaviour, not a defect",
        );
    };

    post(
        api,
        "/api/falsification/defects",
        json!({
            "component": component,
            "claimed_guarantee": guarantee,
            "artifact_ref": str_field(params, "artifact_ref"),
            "seed": params.get("seed").and_then(Value::as_u64),
            "expected": str_field(params, "expected"),
            "observed": str_field(params, "observed"),
            "steps": steps_field(params),
        }),
    )
    .await
}

/// `check_tradeable_frontier` — may this horizon be searched at all.
pub async fn check_tradeable_frontier(api: &ApiClient, params: &Value) -> Value {
    let Some(instrument_id) = str_field(params, "instrument_id") else {
        return missing("instrument_id");
    };
    let Some(holding_bars) = params.get("holding_bars").and_then(Value::as_u64) else {
        return missing("holding_bars");
    };

    post(
        api,
        "/api/research/tradeable-frontier",
        json!({
            "instrument_id": instrument_id,
            "holding_bars": holding_bars,
            "timeframe": str_field(params, "timeframe").unwrap_or_else(|| "1m".into()),
            "venue": str_field(params, "venue"),
            "fee_tier": str_field(params, "fee_tier"),
            "execution_style": str_field(params, "execution_style")
                .unwrap_or_else(|| "taker_both".into()),
            "start": str_field(params, "start"),
            "end": str_field(params, "end"),
        }),
    )
    .await
}

/// `run_oracle_test` — is the cost model charging what it claims.
pub async fn run_oracle_test(api: &ApiClient, params: &Value) -> Value {
    let Some(instrument_id) = str_field(params, "instrument_id") else {
        return missing("instrument_id");
    };
    post(
        api,
        "/api/research/oracle-test",
        json!({
            "instrument_id": instrument_id,
            "timeframe": str_field(params, "timeframe").unwrap_or_else(|| "1m".into()),
            "venue": str_field(params, "venue"),
            "fee_tier": str_field(params, "fee_tier"),
            "execution_style": str_field(params, "execution_style")
                .unwrap_or_else(|| "taker_both".into()),
            "start": str_field(params, "start"),
            "end": str_field(params, "end"),
        }),
    )
    .await
}

// ── evidence (ADR-P7-05) ─────────────────────────────────────────────────────

/// `get_trade_list` — the fills, with concentration measured.
pub async fn get_trade_list(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    post(
        api,
        "/api/evidence/trades",
        json!({
            "run_id": run_id,
            "top_n": params.get("top_n").and_then(Value::as_u64).unwrap_or(6),
        }),
    )
    .await
}

/// `probe_data_quality` — scan bars for the defects that make a backtest lie.
pub async fn probe_data_quality(api: &ApiClient, params: &Value) -> Value {
    let Some(instrument_id) = str_field(params, "instrument_id") else {
        return missing("instrument_id");
    };
    post(
        api,
        "/api/evidence/data-quality",
        json!({
            "instrument_id": instrument_id,
            "timeframe": str_field(params, "timeframe").unwrap_or_else(|| "1m".into()),
            "start": str_field(params, "start"),
            "end": str_field(params, "end"),
        }),
    )
    .await
}

/// `find_similar_trials` — what the ledger already knows.
pub async fn find_similar_trials(api: &ApiClient, params: &Value) -> Value {
    if str_field(params, "config_hash").is_none() && str_field(params, "experiment_id").is_none() {
        return crate::tool_error::refuse(
            "no_selector",
            "neither `config_hash` nor `experiment_id` was given, so there is nothing to look up.",
            "supply `config_hash` to find exact repeats of a configuration, or `experiment_id` to find neighbours in the same line of work",
        );
    }
    post(
        api,
        "/api/evidence/similar-trials",
        json!({
            "config_hash": str_field(params, "config_hash"),
            "experiment_id": str_field(params, "experiment_id"),
            "limit": params.get("limit").and_then(Value::as_u64).unwrap_or(20),
        }),
    )
    .await
}

/// One declaration per tool (AGENT-006 §2.3; Set S tasks S-1.1, S-1.4).
///
/// Filing a report is a `Write`: it appends a row nothing can edit afterwards.
/// It is emphatically **not** `Destructive` — an agent that hesitates to report a
/// defect because the tool is flagged dangerous is an agent that exploits the
/// defect instead, which is the failure the whole mechanism exists to prevent.
///
/// The two reports declare `TrialKind::Falsification` because
/// `audit.selection_bias_verdict` folds an attack on a strategy into N_eff
/// differently from a search for one. `check_tradeable_frontier` and
/// `run_oracle_test` only measure and spend nothing; that is the point of asking
/// them *before* a search rather than after it disappoints.
#[must_use]
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{ToolSpec, TrialKind};
    use crate::taxonomy::Risk;

    // The five fields a `Reproduction` needs. Shared, because a refutation that
    // cannot be replayed is an opinion, and the two report tools must ask for
    // the same evidence or the weaker one becomes the one everybody uses.
    let repro = |extra: Value, mut required: Vec<&'static str>| {
        required.extend(["artifact_ref", "seed", "expected", "observed", "steps"]);
        let mut props = extra;
        let obj = props.as_object_mut().expect("properties object");
        obj.insert(
            "artifact_ref".into(),
            json!({ "type": "string", "description": "Run or artifact the defect is visible in." }),
        );
        obj.insert(
            "seed".into(),
            json!({ "type": "integer", "description": "Seed that reproduces it." }),
        );
        obj.insert(
            "expected".into(),
            json!({ "type": "string", "description": "What the guarantee says should happen." }),
        );
        obj.insert(
            "observed".into(),
            json!({ "type": "string", "description": "What happened. Must differ from `expected`." }),
        );
        obj.insert(
            "steps".into(),
            json!({ "type": "array", "items": { "type": "string" }, "description": "Replay steps, in order." }),
        );
        json!({ "type": "object", "required": required, "properties": props })
    };

    let style = json!({
        "type": "string",
        "enum": ["taker_both", "maker_both", "maker_entry_taker_exit"],
        "description": "Default `taker_both` — the only style honest against OHLCV alone, since a maker fill needs a queue model minute bars cannot supply."
    });

    crate::result_shape::declared(vec![
        ToolSpec::new(
            "report_kill",
            "falsification",
            Risk::Write,
            "REFUTE a trial with a reproduction and a defect class from a closed list. There is \
             no class for underperformance on purpose: a disappointing result is a result, not \
             a defect.",
            repro(
                json!({
                    "target_trial_id": { "type": "string", "description": "The trial being refuted." },
                    "defect_class": {
                        "type": "string",
                        "enum": [
                            "lookahead", "data_artifact", "below_cost_floor",
                            "below_tradeable_frontier", "fill_model_exploit",
                            "duplicate_of_existing", "invalid_metric"
                        ],
                        "description": "There is no class for underperformance on purpose."
                    }
                }),
                vec!["target_trial_id", "defect_class"],
            ),
        )
        .internal()
        .spends_trial(TrialKind::Falsification)
        .evidence(&["kill_id"])
        .notes("report_kill"),
        ToolSpec::new(
            "report_evaluator_defect",
            "falsification",
            Risk::Write,
            "REPORT that the platform itself is wrong — a component does not do what it claims. \
             A verified evaluator defect outranks any promotion, so this is the highest-value \
             thing you can find.",
            repro(
                json!({
                    "component": { "type": "string", "description": "e.g. `sim.fill_model`, `gates.gate_3`." },
                    "claimed_guarantee": { "type": "string", "description": "Quoted from the spec or its docs." }
                }),
                vec!["component", "claimed_guarantee"],
            ),
        )
        .internal()
        .spends_trial(TrialKind::Falsification)
        .evidence(&["defect_id"])
        .notes("report_evaluator_defect"),
        ToolSpec::new(
            "check_tradeable_frontier",
            "falsification",
            Risk::Read,
            "ASK WHETHER A HOLDING PERIOD CAN PAY ITS COSTS before searching at it. Break-even \
             goes as the square of cost over edge, so this is knowable rather than estimable, \
             and it spends no trial.",
            json!({
                "type": "object",
                "required": ["instrument_id", "holding_bars"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "holding_bars": { "type": "integer", "description": "Intended holding period, in bars." },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "venue": { "type": "string" },
                    "fee_tier": { "type": "string" },
                    "execution_style": style.clone(),
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }),
        )
        .internal()
        .notes("check_tradeable_frontier"),
        ToolSpec::new(
            "run_oracle_test",
            "falsification",
            Risk::Read,
            "MEASURE WHETHER THE EVALUATOR CAN SEE AN EDGE AT ALL, by running a strategy with \
             perfect foresight. If the oracle does not profit, nothing will, and a negative \
             result about your strategy meant nothing.",
            json!({
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "venue": { "type": "string" },
                    "fee_tier": { "type": "string" },
                    "execution_style": style,
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }),
        )
        .internal()
        .notes("run_oracle_test"),
        ToolSpec::new(
            "get_trade_list",
            "falsification",
            Risk::Read,
            "ISOLATE THE TRADES A RESULT RESTS ON. Returns the top-N contributors and what the \
             result becomes without them; a Sharpe that lives in six bars is a defect class, \
             not a strategy.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": { "type": "string" },
                    "top_n": { "type": "integer", "description": "Default 6." }
                }
            }),
        )
        .internal()
        .evidence(&["run_id"])
        .notes("get_trade_list"),
        ToolSpec::new(
            "probe_data_quality",
            "falsification",
            Risk::Read,
            "ASK WHETHER THE BARS THEMSELVES ARE THE DEFECT over a window — gaps, repeats, \
             frozen prices, impossible moves. Narrow it to the window `get_trade_list` pointed \
             you at.",
            json!({
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }),
        )
        .internal()
        .notes("probe_data_quality"),
        ToolSpec::new(
            "find_similar_trials",
            "falsification",
            Risk::Read,
            "FIND WHETHER THIS HAS BEEN TRIED. An exact config repeat means the earlier result \
             already answers you; a cluster of neighbours means the family's N_eff is larger \
             than its trial count suggests.",
            json!({
                "type": "object",
                "properties": {
                    "config_hash": { "type": "string", "description": "Find exact repeats." },
                    "experiment_id": { "type": "string", "description": "Find neighbours." },
                    "limit": { "type": "integer", "description": "1-200, default 20." }
                }
            }),
        )
        .internal()
        .evidence(&["trial_id"])
        .notes("find_similar_trials"),
        ToolSpec::new(
            "regime_failure_search",
            "falsification",
            Risk::Read,
            "ASK WHICH MARKET REGIME A FINISHED RUN LOSES IN, by the platform's own causal \
             labels, with a multiple-comparisons correction across regimes. Re-reads the run's \
             equity curve; spends no trial. Regimes are not chosen by you.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": { "type": "string" },
                    "min_obs": { "type": "integer", "minimum": 30, "default": 250,
                                 "description": "fewest periods a regime needs to be tested; thinner regimes are listed untested" }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::None)
        .evidence(&["run_id"])
        .summary(
            "worst regime {worst}; failure found: {failure_found}; {n_tested} tested, {n_untested} too thin to test",
            &[("worst", ""), ("failure_found", ""), ("n_tested", "regimes"), ("n_untested", "regimes")],
        ),
        ToolSpec::new(
            "parameter_fragility_map",
            "falsification",
            Risk::Read,
            "ASK HOW KNIFE-EDGED A FINISHED SWEEP'S WINNER IS: the share of its parameters with a \
             cliff or no plateau, beside the neighbourhood verdict. Reads the sweep's own \
             surface; runs nothing and spends no trial.",
            json!({
                "type": "object",
                "required": ["sweep_id"],
                "properties": { "sweep_id": { "type": "string" } }
            }),
        )
        .internal()
        .spends_trial(TrialKind::None)
        .evidence(&["sweep_id"])
        .summary(
            "fragility index {fragility_index}: {n_knife_edge} of {n_params} parameters knife-edge; neighbourhood plateau {neighbourhood_plateau}",
            &[("fragility_index", ""), ("n_knife_edge", "params"), ("n_params", "params"), ("neighbourhood_plateau", "")],
        ),
        ToolSpec::new(
            "timing_jitter_sensitivity",
            "falsification",
            Risk::Write,
            "ASK WHETHER A V3 STRATEGY'S EDGE SURVIVES BEING LATE: the same experiment run with \
             every decided order held 0..k bars before it reaches the venue. Each delay is a \
             counted look; the delays are not yours to choose beyond the maximum.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": {
                    "experiment_id": { "type": "string" },
                    "max_delay_bars": { "type": "integer", "minimum": 1, "maximum": 12, "default": 3,
                                        "description": "bars of the experiment's evaluation resolution; delays 0..=this are run" },
                    "metric": { "type": "string", "enum": ["sharpe", "sortino", "calmar", "cagr", "total_return", "detrended_sharpe", "expectancy"], "default": "sharpe" },
                    "study_id": { "type": "string", "description": "default `timing-jitter-<max>`" }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::Falsification)
        .evidence(&["study_id"])
        .max_calls_per_run(3)
        .summary(
            "timing {verdict}: {baseline} on time, {at_one_bar} one bar late, {at_max} at {max_delay}",
            &[("verdict", ""), ("baseline", ""), ("at_one_bar", ""), ("at_max", ""), ("max_delay", "bars")],
        ),
        ToolSpec::new(
            "ablation_sweep",
            "falsification",
            Risk::Write,
            "ASK WHICH RULES OF A V1 STRATEGY EARN THEIR PLACE: runs it with each condition and \
             each exit rule left out and classifies the change as load-bearing, redundant or \
             harmful against a paired bootstrap interval. Saves each variant as a draft strategy \
             named `<id>--ablate-<n>`; each run is a counted look.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": {
                    "experiment_id": { "type": "string" },
                    "metric": { "type": "string", "enum": ["sharpe", "sortino", "calmar", "cagr", "total_return", "detrended_sharpe", "expectancy"], "default": "sharpe" },
                    "study_id": { "type": "string", "description": "default `ablation`" }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::Falsification)
        .evidence(&["study_id"])
        .max_calls_per_run(2)
        .summary(
            "{n_load_bearing} of {n_components} load-bearing, {n_redundant} redundant, {n_harmful} harmful, {n_inconclusive} inconclusive",
            &[("n_load_bearing", "components"), ("n_components", "components"), ("n_redundant", "components"), ("n_harmful", "components"), ("n_inconclusive", "components")],
        ),
    ])
}

// ── S-3.13: the two re-read falsification tools ──────────────────────────────
//
// Neither dispatches anything or reads new data, so neither spends a trial
// (ADR-D-12). The arithmetic is `research::{regime_search, fragility}`, reached
// through `/api/agent/falsification/*`; this crate stays a thin client.

/// `regime_failure_search` -- the run's worst regime by the platform's own
/// causal labels, with a multiple-comparisons correction across regimes.
pub async fn regime_failure_search(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    let mut path = format!(
        "/api/agent/falsification/regime-failure/{}",
        crate::tools::market::urlencode(&run_id)
    );
    if let Some(m) = params.get("min_obs").and_then(Value::as_u64) {
        path.push_str(&format!("?min_obs={m}"));
    }
    let resp = match api.get(&path).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let s = resp.get("search").cloned().unwrap_or(Value::Null);
    json!({
        "summary_fields": {
            "worst": s.get("worst").cloned().unwrap_or(Value::Null),
            "failure_found": s.get("failure_found").cloned().unwrap_or(Value::Null),
            "n_tested": s.get("n_tested").cloned().unwrap_or(Value::Null),
            "n_untested": s.get("untested").and_then(Value::as_array).map_or(0, Vec::len),
        },
        "detail": resp,
        "n": s.get("n_periods").cloned().unwrap_or(Value::Null),
        "ids": {},
    })
}

/// `parameter_fragility_map` -- how many of a finished sweep's dials the result
/// is balanced on, as one index beside the neighbourhood verdict.
pub async fn parameter_fragility_map(api: &ApiClient, params: &Value) -> Value {
    let Some(sweep_id) = str_field(params, "sweep_id") else {
        return missing("sweep_id");
    };
    let resp = match api
        .get(&format!(
            "/api/agent/falsification/fragility/{}",
            crate::tools::market::urlencode(&sweep_id)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let f = resp.get("fragility").cloned().unwrap_or(Value::Null);
    json!({
        "summary_fields": {
            "fragility_index": f.get("fragility_index").cloned().unwrap_or(Value::Null),
            "n_knife_edge": f.get("n_knife_edge").cloned().unwrap_or(Value::Null),
            "n_params": f.get("n_params").cloned().unwrap_or(Value::Null),
            "neighbourhood_plateau": f.get("neighbourhood_plateau").cloned().unwrap_or(Value::Null),
        },
        "detail": resp,
        "ids": {},
    })
}
// ── S-3.13: timing jitter and ablation ───────────────────────────────────────
//
// Both run real backtests through the one door that counts them (`run_study`),
// so neither can spend a look the ledger does not see. What these add is the
// question, fixed -- the delays and the variants are the platform's, not the
// caller's -- and the reading of the answer.

/// Metrics whose sign means "there is an edge to lose": positive is good, zero is
/// none. `max_drawdown` and `profit_factor` do not fit that reading.
const EDGE_METRICS: [&str; 7] = [
    "sharpe",
    "sortino",
    "calmar",
    "cagr",
    "total_return",
    "detrended_sharpe",
    "expectancy",
];

fn metric_arg(params: &Value) -> Result<String, Value> {
    let m = str_field(params, "metric").unwrap_or_else(|| "sharpe".to_string());
    if EDGE_METRICS.contains(&m.as_str()) {
        Ok(m)
    } else {
        Err(crate::tool_error::refuse_extra(
            "unsupported_metric",
            format!("metric `{m}` is not one this tool can compute."),
            format!("set metric to one of {}", EDGE_METRICS.join(", ")),
            json!({ "got": m }),
        ))
    }
}

/// Retained share at which a lag is called tolerable. A platform constant: a caller
/// who can set it can set it until the strategy passes.
pub const JITTER_TOLERANT_RETAINED: f64 = 0.8;

/// Read a timing-jitter distribution (member order = delay order).
///
/// * `no_edge_to_delay` -- the on-time metric is not positive, so lateness has
///   nothing to take away and the question does not apply;
/// * `timing_fragile` -- the edge is gone (metric <= 0) by one bar of delay;
/// * `timing_tolerant` -- at the longest delay at least [`JITTER_TOLERANT_RETAINED`]
///   of the on-time metric is still there;
/// * `timing_sensitive` -- in between;
/// * `unmeasured` -- a delay's run did not complete, so the curve has a hole. A
///   curve with a hole is not read at all: skipping the failed delay would show the
///   survivors, which are the delays that flattered it.
#[must_use]
pub fn read_timing_curve(dist: &[f64], bars: &[u32]) -> Value {
    if dist.len() != bars.len() || bars.first() != Some(&0) || dist.iter().any(|v| !v.is_finite()) {
        return json!({
            "verdict": "unmeasured",
            "reason": format!(
                "{} of {} delays produced a finite metric; a curve with a missing delay is not \
                 read, because the delays that completed are the ones that flatter it. A v1 \
                 strategy cannot be delayed at all: its simulator has no order-hold path",
                dist.iter().filter(|v| v.is_finite()).count().min(bars.len()),
                bars.len()
            ),
        });
    }
    let base = dist[0];
    let curve: Vec<Value> = bars
        .iter()
        .zip(dist)
        .map(|(b, v)| {
            json!({
                "delay_bars": b,
                "metric": v,
                "retained": if base > 0.0 { json!(v / base) } else { Value::Null },
            })
        })
        .collect();
    let at = |bar: u32| bars.iter().position(|b| *b == bar).map(|i| dist[i]);
    let last = *dist.last().unwrap_or(&base);
    let max_delay = *bars.last().unwrap_or(&0);
    let first_dead = bars
        .iter()
        .zip(dist)
        .find(|(b, v)| **b > 0 && **v <= 0.0)
        .map(|(b, _)| *b);
    let verdict = if base <= 0.0 {
        "no_edge_to_delay"
    } else if first_dead.is_some_and(|b| b <= 1) {
        "timing_fragile"
    } else if last / base >= JITTER_TOLERANT_RETAINED {
        "timing_tolerant"
    } else {
        "timing_sensitive"
    };
    json!({
        "verdict": verdict,
        "baseline": base,
        "at_one_bar": at(1),
        "at_max": last,
        "max_delay": max_delay,
        "edge_gone_at_delay_bars": first_dead,
        "curve": curve,
        "tolerant_if_retained_at_least": JITTER_TOLERANT_RETAINED,
    })
}

/// `timing_jitter_sensitivity` -- one `timing_jitter` study over delays 0..=k.
pub async fn timing_jitter_sensitivity(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    let max_delay = params.get("max_delay_bars").map_or(Some(3), Value::as_u64);
    let Some(max_delay) = max_delay.filter(|m| (1..=12).contains(m)) else {
        return crate::tool_error::refuse(
            "invalid_max_delay_bars",
            "max_delay_bars is outside 1 to 12.",
            "set max_delay_bars to an integer from 1 to 12",
        );
    };
    let metric = match metric_arg(params) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let bars: Vec<u32> = (0..=u32::try_from(max_delay).unwrap_or(3)).collect();
    let study_id =
        str_field(params, "study_id").unwrap_or_else(|| format!("timing-jitter-{max_delay}"));
    let body = json!({
        "study_id": study_id,
        "kind": "timing_jitter",
        "vary": { "vary": "fill_delays", "bars": bars },
        "metric": metric,
        "question": format!(
            "Does the edge survive every decided order reaching the venue 0 to {max_delay} late?"
        ),
    });
    let view = match api
        .post(
            &format!(
                "/api/backtest/experiments/{}/studies",
                crate::tools::market::urlencode(&experiment_id)
            ),
            body,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let dist: Vec<f64> = view
        .pointer("/distribution/dist")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let read = read_timing_curve(&dist, &bars);
    json!({
        "summary_fields": {
            "verdict": read["verdict"],
            "baseline": read.get("baseline").cloned().unwrap_or(Value::Null),
            "at_one_bar": read.get("at_one_bar").cloned().unwrap_or(Value::Null),
            "at_max": read.get("at_max").cloned().unwrap_or(Value::Null),
            "max_delay": max_delay,
        },
        "detail": { "reading": read, "metric": metric, "study": view, "delay_unit": "bars of the experiment's evaluation resolution" },
        "n": bars.len(),
        "ids": { "study_id": study_id },
    })
}

/// Tally the per-component verdicts the classify route returned.
#[must_use]
pub fn tally_ablation(components: &[Value]) -> Value {
    let count = |v: &str| {
        components
            .iter()
            .filter(|c| c.pointer("/result/verdict").and_then(Value::as_str) == Some(v))
            .count()
    };
    json!({
        "n_components": components.len(),
        "n_load_bearing": count("load_bearing"),
        "n_redundant": count("redundant"),
        "n_harmful": count("harmful"),
        "n_inconclusive": count("inconclusive"),
    })
}

/// `ablation_sweep` -- variants as draft strategies, one study, the verdicts.
pub async fn ablation_sweep(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    let metric = match metric_arg(params) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let study_id = str_field(params, "study_id").unwrap_or_else(|| "ablation".to_string());
    let exp = crate::tools::market::urlencode(&experiment_id);

    // 1. The plan: what to leave out, and the id each variant is stored under.
    let plan = match api
        .get(&format!("/api/agent/falsification/ablation/{exp}/variants"))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let base_ref = plan["strategy_ref"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let variants = plan["variants"].as_array().cloned().unwrap_or_default();
    if base_ref.is_empty() || variants.is_empty() {
        return crate::tool_error::refuse_extra(
            "no_plan",
            "the platform returned an ablation plan with no base strategy or no variants.",
            "check `strategy_id` names a stored strategy with at least one removable rule, then call again",
            json!({ "detail": plan }),
        );
    }

    // 2. Register each variant as a draft strategy through the ordinary door. A
    // retry finds the slug already has a head, and the lineage parameter is then
    // refused as ambiguous; the retry without it re-saves the same document, which
    // is a no-op, so a second call does not litter the table.
    let mut registered: Vec<Value> = Vec::new();
    let mut refs = vec![Value::String(base_ref.clone())];
    let mut components: Vec<String> = Vec::new();
    for v in &variants {
        let id = v["strategy_id"].as_str().unwrap_or_default();
        let component = v["component"].as_str().unwrap_or_default();
        let def = v["definition"].clone();
        let with_parent = format!(
            "/api/strategies?parent_strategy_id={}",
            crate::tools::market::urlencode(&base_ref)
        );
        let mut res = api.post(&with_parent, def.clone()).await;
        if res.is_err() {
            res = api.post("/api/strategies", def).await;
        }
        match res {
            Ok(_) => {
                registered.push(json!({"component": component, "strategy_id": id}));
                refs.push(Value::String(id.to_string()));
                components.push(component.to_string());
            }
            Err(e) => {
                return crate::tool_error::refuse_extra(
                    "variant_not_registered",
                    format!("variant `{id}` (without `{component}`) could not be registered; nothing was run and no look was spent."),
                    "call ablation_sweep again; a retry finds the variants already registered and continues",
                    json!({
                        "component": component,
                        "strategy_id": id,
                        "detail": e.to_tool_error(),
                        "registered_so_far": registered,
                    }),
                );
            }
        }
    }

    // 3. One study over [baseline, variants..]: each member a counted look.
    let study = match api
        .post(
            &format!("/api/backtest/experiments/{exp}/studies"),
            json!({
                "study_id": study_id,
                "kind": "ablation",
                "vary": { "vary": "strategy_refs", "refs": refs },
                "metric": metric,
                "question": "Which of this strategy's rules earn their place?",
            }),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            return crate::tool_error::refuse_extra(
                "study_failed",
                "the ablation study could not be started; the variants are registered and no look was spent.",
                "call ablation_sweep again; the variants already exist and the study is retried",
                json!({ "detail": e.to_tool_error(), "registered": registered }),
            )
        }
    };

    // 4. The verdicts, from the members' own equity curves.
    let classified = match api
        .post(
            &format!("/api/agent/falsification/ablation/{exp}/classify"),
            json!({"study_id": study_id, "components": components}),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return crate::tool_error::refuse_extra(
            "classification_failed",
            "the study ran but its members could not be classified; the looks are spent.",
            "do not re-run the study; read `study` and report the members' metrics unclassified",
            json!({ "detail": e.to_tool_error(), "registered": registered, "study": study }),
        ),
    };
    let rows = classified["components"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let tally = tally_ablation(&rows);
    json!({
        "summary_fields": tally,
        "detail": {
            "classification": classified,
            "registered": registered,
            "metric": metric,
            "study": study,
            "reading": "load_bearing: removing it makes the strategy worse by more than noise; \
                        redundant: indistinguishable from noise either way; harmful: removing \
                        it helps. The interval is a paired stationary bootstrap with a derived \
                        block length and a fixed seed.",
        },
        "n": rows.len(),
        "ids": { "study_id": study_id },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Value {
        json!({
            "artifact_ref": "run_1",
            "seed": 42,
            "expected": "shifting signals back one bar leaves the Sharpe unchanged",
            "observed": "the Sharpe inverts from +2.9 to -2.7",
            "steps": ["re-run with signal_shift=1", "compare net_sharpe"]
        })
    }

    fn err(params: Value) -> Value {
        check_reproduction(&params).unwrap_err()
    }

    #[test]
    fn a_complete_reproduction_passes_preflight() {
        assert!(check_reproduction(&good()).is_ok());
    }

    /// The degenerate report. The agent is told *why*, in the words that make
    /// the distinction learnable rather than just enforced.
    #[test]
    fn a_report_that_refutes_nothing_is_rejected_with_the_reason() {
        let mut p = good();
        p["expected"] = json!("sharpe is 0.4");
        p["observed"] = json!("sharpe is 0.4");
        let e = err(p);
        assert_eq!(e["error"]["code"], "not_falsifiable");
        assert!(e["error"]["message"]
            .as_str()
            .unwrap()
            .contains("A low score is not a kill"));
    }

    #[test]
    fn whitespace_does_not_make_a_report_falsifiable() {
        let mut p = good();
        p["expected"] = json!("  same  ");
        p["observed"] = json!("same");
        assert_eq!(err(p)["error"]["code"], "not_falsifiable");
    }

    #[test]
    fn a_report_without_steps_is_rejected() {
        let mut p = good();
        p["steps"] = json!([]);
        assert_eq!(err(p.clone())["error"]["code"], "no_steps");
        p["steps"] = json!(["   ", ""]);
        assert_eq!(err(p)["error"]["code"], "no_steps");
    }

    #[test]
    fn a_report_without_a_seed_is_rejected_and_says_why() {
        let mut p = good();
        p["seed"] = Value::Null;
        let e = err(p);
        assert_eq!(e["field"], "seed");
        assert!(e["error"]["message"].as_str().unwrap().contains("anecdote"));
    }

    #[test]
    fn every_reproduction_field_is_required() {
        for field in ["artifact_ref", "expected", "observed"] {
            let mut p = good();
            p[field] = json!("  ");
            assert_eq!(err(p)["field"], field);
        }
    }

    // ── the declarations, and the prose that moved out of them ───────────────
    //
    // These seven descriptions used to carry 6.4 KB of guidance between them,
    // and the catalogue is sent on every model call. S-1.4 moved that guidance
    // to `## Tool notes` in the authoring guide, which a model fetches once.
    //
    // The tests moved with it. Each one below asserts the same claim it always
    // did — it just asserts it where the words now live. The point was never
    // that a particular string sat in a particular field; it was that the agent
    // is told the thing. A test that checked only the short description would
    // pass while the substance quietly rotted out of the guide.

    fn notes(tool: &str) -> String {
        // Line endings are not part of the claim. `include_str!` reads the working
        // copy, and a checkout under `core.autocrlf` hands back CRLF whatever the
        // file was committed as, so a helper matching the heading plus a newline
        // reports every section missing on one machine and present on another,
        // from identical content. Normalise first, and the test is about the guide.
        let guide = crate::authoring_guide().replace("\r\n", "\n");
        let heading = format!("### {tool}\n");
        let start = guide
            .find(&heading)
            .unwrap_or_else(|| panic!("`### {tool}` must exist in the authoring guide"));
        let rest = &guide[start + heading.len()..];
        let end = rest.find("\n### ").unwrap_or(rest.len());
        // The guide is hard-wrapped; these assertions are about content, so a
        // claim must not become invisible because it straddled a line break.
        rest[..end].split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn described(tool: &str) -> &'static str {
        specs()
            .into_iter()
            .find(|s| s.name == tool)
            .unwrap_or_else(|| panic!("{tool} must be declared"))
            .description
    }

    #[test]
    fn every_tool_is_declared_with_a_schema() {
        let specs = specs();
        assert_eq!(
            specs.len(),
            11,
            "four falsification tools, three evidence tools, the two re-read searches and the two run-and-read tools (S-3.13)"
        );
        for s in &specs {
            assert!(!s.description.is_empty(), "{}", s.name);
            assert_eq!(s.input_schema["type"], "object");
            // Not every tool has a required field — `find_similar_trials` takes
            // either of two selectors and neither alone is mandatory. What every
            // tool must have is properties, or the model has nothing to fill in.
            assert!(
                !s.input_schema["properties"].as_object().unwrap().is_empty(),
                "{} has no properties",
                s.name
            );
            // Every one of these moved prose out, so every one must point at it --
            // except the two re-read searches (S-3.13), which carry no moved prose
            // and whose descriptions are complete as written.
            if matches!(
                s.name,
                "regime_failure_search"
                    | "parameter_fragility_map"
                    | "timing_jitter_sensitivity"
                    | "ablation_sweep"
            ) {
                continue;
            }
            assert_eq!(
                s.notes_section,
                Some(s.name),
                "{} moved prose to the guide and must name the section",
                s.name
            );
        }
    }

    /// The ordering has to reach the model, not just the database. A tool that
    /// does not say "scores above a promotion" leaves the agent to infer that
    /// reporting a bug costs it.
    #[test]
    fn the_defect_tool_tells_the_model_it_outranks_a_promotion() {
        let n = notes("report_evaluator_defect");
        assert!(n.contains("ABOVE a promotion"), "{n}");
        assert!(n.contains("exploiting it silently scores less"), "{n}");
        // The short form must carry the ranking too: it is the half a flat-tier
        // model sees without fetching anything.
        assert!(described("report_evaluator_defect").contains("outranks any promotion"));
    }

    /// The kill tool must spell out what does not count, or the model will file
    /// low Sharpes until the constructor stops it.
    #[test]
    fn the_kill_tool_says_what_does_not_count() {
        let n = notes("report_kill");
        assert!(n.contains("Does NOT count"), "{n}");
        assert!(n.contains("Sharpe is only 0.4"), "{n}");
        assert!(described("report_kill").contains("no class for underperformance"));
    }

    /// There is no `underperforms` class, and there must never be one.
    #[test]
    fn the_defect_class_enum_contains_no_performance_threshold() {
        let s = specs();
        let d = s.iter().find(|s| s.name == "report_kill").unwrap();
        let classes = d.input_schema["properties"]["defect_class"]["enum"]
            .as_array()
            .expect("the closed list is in the schema, not only in the guide");
        assert_eq!(classes.len(), 7);
        for c in classes {
            let s = c.as_str().unwrap();
            for banned in ["underperform", "sharpe", "return", "loss", "worse"] {
                assert!(!s.contains(banned), "{s} names a threshold, not a defect");
            }
            // And the guide must name every class the schema accepts, or the
            // model is offered a value with nowhere to read what it means.
            assert!(
                notes("report_kill").contains(s),
                "`{s}` is accepted by the schema and absent from the guide"
            );
        }
    }

    /// The three evidence tools close a hole the falsification sprint left: a
    /// defect vocabulary with no way to gather evidence for it. Each must point
    /// at the next step rather than reading as a verdict on its own.
    #[test]
    fn the_evidence_tools_say_what_they_do_not_prove() {
        let trades = notes("get_trade_list");
        assert!(
            trades.contains("Concentration alone is not a kill"),
            "{trades}"
        );
        assert!(
            trades.contains("probe_data_quality"),
            "it must point at the next step: {trades}"
        );

        let quality = notes("probe_data_quality");
        assert!(quality.contains("NOT a clean bill of health"), "{quality}");
        assert!(quality.contains("ZERO VOLUME"), "{quality}");

        let similar = notes("find_similar_trials");
        assert!(similar.contains("ALREADY BEEN EVALUATED"), "{similar}");
        assert!(
            similar.contains("effective trial count"),
            "the reason repetition matters is N_eff, and the model has to be told: {similar}"
        );
    }

    #[test]
    fn the_frontier_tool_defaults_to_the_only_honest_execution_style() {
        let s = specs();
        let d = s
            .iter()
            .find(|s| s.name == "check_tradeable_frontier")
            .unwrap();
        let style = &d.input_schema["properties"]["execution_style"];
        assert_eq!(style["enum"][0], "taker_both");
        // The *reason* stays in the schema rather than the guide: it is read at
        // the moment the field is filled in, which is the moment it matters.
        assert!(style["description"]
            .as_str()
            .unwrap()
            .contains("queue model"));
    }

    /// The oracle test is about the simulator. A description that reads as a
    /// strategy tool would get it used as one.
    #[test]
    fn the_oracle_tool_says_it_tests_the_simulator_and_what_failure_means() {
        let n = notes("run_oracle_test");
        assert!(n.contains("test of the SIMULATOR"), "{n}");
        assert!(n.contains("must LOSE money"), "{n}");
        assert!(
            n.contains("report_evaluator_defect"),
            "route failure onward: {n}"
        );
        assert!(described("run_oracle_test").contains("EVALUATOR CAN SEE AN EDGE"));
    }

    // ── S-3.13: timing jitter and ablation ───────────────────────────────────

    #[test]
    fn a_curve_that_dies_at_one_bar_is_fragile_and_one_that_holds_is_tolerant() {
        let bars = [0, 1, 2, 3];
        let fragile = read_timing_curve(&[1.4, -0.1, -0.5, -0.9], &bars);
        assert_eq!(fragile["verdict"], "timing_fragile");
        assert_eq!(fragile["edge_gone_at_delay_bars"], 1);
        let tolerant = read_timing_curve(&[1.4, 1.35, 1.3, 1.2], &bars);
        assert_eq!(tolerant["verdict"], "timing_tolerant");
        let sensitive = read_timing_curve(&[1.4, 1.1, 0.9, 0.6], &bars);
        assert_eq!(sensitive["verdict"], "timing_sensitive");
        assert!(sensitive["curve"][3]["retained"].as_f64().unwrap() < 0.5);
    }

    #[test]
    fn no_edge_on_time_means_the_question_does_not_apply_and_a_hole_is_not_read() {
        let none = read_timing_curve(&[-0.2, -0.3, -0.4], &[0, 1, 2]);
        assert_eq!(none["verdict"], "no_edge_to_delay");
        assert!(
            none["curve"][1]["retained"].is_null(),
            "a share of nothing is not a number"
        );
        // A failed delay leaves fewer values than delays: refused, not skipped.
        let hole = read_timing_curve(&[1.4, 1.3], &[0, 1, 2]);
        assert_eq!(hole["verdict"], "unmeasured");
        assert!(hole["reason"].as_str().unwrap().contains("order-hold"));
        // And a curve that does not start at the on-time baseline is not read either.
        assert_eq!(
            read_timing_curve(&[1.0, 1.0], &[1, 2])["verdict"],
            "unmeasured"
        );
    }

    #[test]
    fn the_jitter_and_ablation_tools_refuse_a_metric_with_no_edge_reading() {
        assert!(metric_arg(&json!({"metric": "sharpe"})).is_ok());
        assert_eq!(metric_arg(&json!({})).unwrap(), "sharpe");
        let e = metric_arg(&json!({"metric": "max_drawdown"})).unwrap_err();
        assert_eq!(e["error"]["code"], "unsupported_metric");
    }

    #[test]
    fn the_ablation_tally_counts_each_verdict_and_an_unrun_variant_as_inconclusive() {
        let rows = vec![
            json!({"result": {"verdict": "load_bearing"}}),
            json!({"result": {"verdict": "redundant"}}),
            json!({"result": {"verdict": "redundant"}}),
            json!({"result": {"verdict": "inconclusive", "reason": "the variant's run did not complete"}}),
        ];
        let t = tally_ablation(&rows);
        assert_eq!(t["n_components"], 4);
        assert_eq!(t["n_load_bearing"], 1);
        assert_eq!(t["n_redundant"], 2);
        assert_eq!(t["n_inconclusive"], 1);
        assert_eq!(t["n_harmful"], 0);
    }

    #[test]
    fn both_tools_are_declared_as_falsification_trials_and_named_in_the_catalogue() {
        for name in ["timing_jitter_sensitivity", "ablation_sweep"] {
            let s = specs().into_iter().find(|s| s.name == name).expect(name);
            assert_eq!(
                s.spends_trial,
                crate::spec::TrialKind::Falsification,
                "{name} runs backtests"
            );
            assert!(s.max_calls_per_run.is_some(), "{name} must be bounded");
        }
    }
}
