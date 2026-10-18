//! The honesty arithmetic (AGENT-006 §5.4; Set S tasks S-3.6, S-3.7).
//!
//! Four tools, and none of them computes anything this crate owns. Every number
//! here is produced by the code the **gate** runs — `backtest::stats` for the
//! deflated Sharpe, `ledger::neff::NEff` for the trial count, the experiment
//! vault for the holdout — because a tool that agreed with the gate by
//! coincidence is a tool that will disagree with it eventually, and the
//! disagreement would surface as a strategy that passed an audit and failed a
//! promotion for reasons nobody could reconstruct.
//!
//! # Why the trial count cannot come from the caller
//!
//! The deflated Sharpe is the observed Sharpe discounted by how many things were
//! tried. An agent that supplied its own trial count could supply a small one,
//! and the arithmetic that exists to expose selection bias would be reporting
//! whatever the selector preferred. So `trials` is `NEff`, which has no public
//! constructor and can only be minted by a ledger (§12.4, INV-22) — the same
//! value `gate3` uses, obtained the same way.
//!
//! # The holdout has one door
//!
//! `spend_holdout_look` is the only tool that reaches holdout rows, it carries
//! the `SealedHoldout` envelope so it suspends for a human on every tier, and it
//! is capped at one call per run. `check_holdout_budget` is how an agent asks
//! what it has left *without* spending it — which is the whole point, because an
//! agent that can only discover the budget by exhausting it will exhaust it.

use serde_json::{json, Value};

use crate::spec::{Envelope, ToolSpec, TrialKind};
use crate::taxonomy::Risk;
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

async fn post(api: &ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

// ── S-3.6: the discount for having looked ────────────────────────────────────

pub async fn deflated_sharpe(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    post(
        api,
        "/api/agent/audit/deflated-sharpe",
        json!({
            "run_id": run_id,
            "experiment_id": params.get("experiment_id").cloned(),
        }),
    )
    .await
}

pub async fn min_track_record(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    // No `profile` is forwarded, and the schema has no field for one: which gate
    // profile applies is the platform's decision, because a caller who names the
    // profile names the one with no calendar floor. See the route.
    post(
        api,
        "/api/agent/audit/min-track-record",
        json!({ "run_id": run_id }),
    )
    .await
}

// ── S-3.7: the one door ──────────────────────────────────────────────────────

pub async fn check_holdout_budget(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        "/api/agent/audit/holdout-budget",
        json!({ "experiment_id": experiment_id }),
    )
    .await
}

pub async fn spend_holdout_look(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    let Some(reason) = str_field(params, "reason") else {
        return missing("reason");
    };
    post(
        api,
        "/api/agent/audit/holdout-look",
        json!({ "experiment_id": experiment_id, "reason": reason }),
    )
    .await
}

// ── S-3.9: what a family of candidates is worth ──────────────────────────────

pub async fn probability_of_overfit(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        "/api/agent/audit/probability-of-overfit",
        json!({
            "experiment_id": experiment_id,
            "n_periods": params.get("n_periods").cloned(),
        }),
    )
    .await
}

pub async fn family_wise_test(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = str_field(params, "experiment_id") else {
        return missing("experiment_id");
    };
    post(
        api,
        "/api/agent/audit/family-wise",
        json!({
            "experiment_id": experiment_id,
            "alpha": params.get("alpha").cloned(),
            "seed": params.get("seed").cloned(),
        }),
    )
    .await
}

// ── S-3.8: does the report match what the platform logged ────────────────────

pub async fn metric_integrity(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    let Some(metric) = str_field(params, "metric") else {
        return missing("metric");
    };
    let Some(claimed) = params.get("claimed_value").and_then(Value::as_f64) else {
        return missing("claimed_value");
    };
    post(
        api,
        "/api/agent/audit/metric-integrity",
        json!({ "run_id": run_id, "metric": metric, "claimed_value": claimed }),
    )
    .await
}

// ── S-3.10: does the strategy read bars it could not have seen ───────────────

/// The columns a strategy reads, from a baseline preview's own feature list.
fn feature_names(preview: &Value) -> Vec<String> {
    preview
        .get("features")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// How far two previews of one strategy disagree: the summed difference in bars
/// on which each signal's condition held and in its firings, plus one if the
/// sampled entry bars differ. Zero means indistinguishable on everything a
/// preview reports. The sampled events are capped at 20 by the preview itself,
/// which `compared_on` says.
fn divergence(a: &Value, b: &Value) -> u64 {
    let stat = |v: &Value, sig: &str, key: &str| -> i64 {
        v.get("signals")
            .and_then(Value::as_array)
            .and_then(|s| {
                s.iter()
                    .find(|x| x.get("signal").and_then(Value::as_str) == Some(sig))
            })
            .and_then(|x| x.get(key))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    let mut names = std::collections::BTreeSet::new();
    for v in [a, b] {
        for s in v.get("signals").and_then(Value::as_array).into_iter().flatten() {
            if let Some(n) = s.get("signal").and_then(Value::as_str) {
                names.insert(n.to_string());
            }
        }
    }
    let mut d: u64 = 0;
    for n in &names {
        d += stat(a, n, "bars_true").abs_diff(stat(b, n, "bars_true"));
        d += stat(a, n, "fires").abs_diff(stat(b, n, "fires"));
    }
    let bars = |v: &Value| -> Vec<(String, i64)> {
        v.get("sample_events")
            .and_then(Value::as_array)
            .map(|e| {
                e.iter()
                    .map(|x| {
                        (
                            x.get("signal").and_then(Value::as_str).unwrap_or("").to_string(),
                            x.get("bar").and_then(Value::as_i64).unwrap_or(-1),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if bars(a) != bars(b) {
        d += 1;
    }
    d
}

fn never_fires_code(v: &Value) -> Option<&str> {
    v.get("never_fires_code").and_then(Value::as_str)
}

/// One column's two probes, as the platform answered them.
pub struct ColumnProbe {
    pub column: String,
    pub shifted: Value,
    pub masked: Value,
}

/// The verdict over a baseline and one probe pair per column. Pure, so the
/// reading rule is tested without a platform.
///
/// The rule, and why it takes two probes: a forward shift that changes nothing
/// means the strategy "was already reading that bar" (`backtest::perturb`) -- but
/// it equally means the column never decided anything. The mask probe separates
/// them. A column is **biased** only when it is decisive (masking it changes the
/// strategy's behaviour) and yet moving it in time does not. A column masking
/// does not move is `not_decisive` and is reported as untestable, never as clean.
#[must_use]
pub fn assess_perturbations(base: &Value, probes: &[ColumnProbe], shift_bars: i64) -> Value {
    let entries = base.get("entries").and_then(Value::as_u64).unwrap_or(0);
    if entries == 0 {
        return crate::tool_error::refuse_extra(
            "baseline_never_fires",
            "the unperturbed strategy produced no entry over this window, so there is \
             nothing for a perturbation to move. \"No divergence\" over a strategy that \
             never fires is about the window, not about lookahead.",
            "widen the window or fix the strategy until preview_signals shows entries",
            json!({ "never_fires_because": base.get("never_fires_because").cloned() }),
        );
    }
    let mut columns = Vec::new();
    let mut biased: Vec<String> = Vec::new();
    let mut max_div: u64 = 0;
    let mut biased_signal_count: u64 = 0;
    let mut inconclusive = 0usize;
    let mut sensitive = 0usize;
    for p in probes {
        if never_fires_code(&p.shifted) == Some("perturbed_column_has_no_value") {
            inconclusive += 1;
            columns.push(json!({
                "column": p.column, "classification": "inconclusive",
                "why": "the shift left the column with no value on any bar, which is a mask by another name",
            }));
            continue;
        }
        let shift_div = divergence(base, &p.shifted);
        let mask_blank = never_fires_code(&p.masked) == Some("perturbed_column_has_no_value");
        let mask_div = if mask_blank { entries.max(1) } else { divergence(base, &p.masked) };
        max_div = max_div.max(shift_div);
        let class = if mask_div == 0 {
            inconclusive += 1;
            "not_decisive"
        } else if shift_div == 0 {
            biased.push(p.column.clone());
            biased_signal_count += mask_div;
            "biased"
        } else {
            sensitive += 1;
            "sensitive"
        };
        columns.push(json!({
            "column": p.column,
            "classification": class,
            "shift_divergence": shift_div,
            "mask_divergence": mask_div,
        }));
    }
    let verdict = if !biased.is_empty() {
        "suspected_lookahead"
    } else if sensitive == 0 {
        "inconclusive"
    } else if inconclusive > 0 {
        "no_pattern_on_tested_columns"
    } else {
        "no_pattern_found"
    };
    json!({
        "summary_fields": {
            "has_bias": !biased.is_empty(),
            "n_columns": probes.len(),
            "biased_indicators": if biased.is_empty() { "none".to_string() } else { biased.join(", ") },
            "verdict": verdict,
        },
        "detail": {
            "has_bias": !biased.is_empty(),
            "biased_indicators": biased,
            "biased_signal_count": biased_signal_count,
            "max_divergence": max_div,
            "verdict": verdict,
            "shift_bars": shift_bars,
            "columns": columns,
            "compared_on": "per-signal fires and bars_true, plus the first 20 entry bars the preview samples",
            "reading": "a column is `biased` when masking it changes the strategy (it is decisive) but moving it in time does not. `not_decisive` columns are untestable here and are never reported as clean.",
            "spends_trial": false,
        },
        "n": probes.len(),
        "ids": {},
    })
}

/// `feature_leakage_scan` -- the static pass over submitted code (S-3.11). Pure and
/// local: it reads no platform state, so it costs no round trip and no trial.
#[must_use]
pub fn feature_leakage_scan(params: &Value) -> Value {
    use strategy_validator::leakage_scan::{scan, Scan, Severity};
    let Some(source) = params.get("source").and_then(Value::as_str) else {
        return missing("source");
    };
    if source.trim().is_empty() {
        return crate::tool_error::refuse(
            "empty_source",
            "`source` is empty, so there is nothing to scan.",
            "pass the code to scan in `source`",
        );
    }
    if source.len() > 200_000 {
        return crate::tool_error::refuse_extra(
            "source_too_large",
            format!("`source` is {} bytes; the scan takes 200000 at most.", source.len()),
            "scan one cell or file at a time (200000 bytes at most)",
            json!({ "got_bytes": source.len() }),
        );
    }
    let r = scan(source);
    let n_blocking = r.blocking();
    let n_flags = r.findings.len() - n_blocking;
    let verdict = if n_blocking > 0 {
        "blocking_findings"
    } else if n_flags > 0 {
        "flags_only"
    } else {
        "no_findings_not_a_clean_bill"
    };
    let findings: Vec<Value> = r
        .findings
        .iter()
        .map(|f| {
            json!({
                "rule": f.rule,
                "severity": f.severity.as_str(),
                "line": f.line,
                "snippet": f.snippet,
                "why": f.why,
            })
        })
        .collect();
    let _ = Severity::Flag;
    json!({
        "summary_fields": {
            "verdict": verdict,
            "n_blocking": n_blocking,
            "n_flags": n_flags,
            "lines_scanned": r.lines_scanned,
        },
        "detail": {
            "findings": findings,
            "labels_recognised": r.labels,
            "limits": Scan::LIMITS,
        },
        "n": r.findings.len(),
        "ids": {},
    })
}

/// `future_perturbation_test` -- re-run a draft with each feature column moved in
/// time and masked, through the free preview route, and diff the entries.
pub async fn future_perturbation_test(api: &ApiClient, params: &Value) -> Value {
    for field in ["definition", "instrument_id", "start", "end"] {
        if params.get(field).is_none() {
            return missing(field);
        }
    }
    let shift = params.get("shift_bars").and_then(Value::as_i64).unwrap_or(1);
    if !(1..=20).contains(&shift) {
        return crate::tool_error::refuse_extra(
            "invalid_shift_bars",
            format!("shift_bars {shift} is outside 1 to 20."),
            "set shift_bars to a whole number of bars from 1 to 20; a zero shift is an unperturbed run",
            json!({ "got": shift }),
        );
    }
    let body = |perturbations: Value| {
        let mut b = json!({
            "definition": params["definition"],
            "instrument_id": params["instrument_id"],
            "start": params["start"],
            "end": params["end"],
            "timeframe": params.get("timeframe").cloned().unwrap_or(json!("1m")),
            "perturbations": perturbations,
        });
        if let Some(p) = params.get("project_id") {
            b["project_id"] = p.clone();
        }
        b
    };
    let preview = |b: Value| async move {
        api.post("/api/agent/strategy/preview", b)
            .await
            .map_err(|e| e.to_tool_error())
    };
    let base = match preview(body(json!([]))).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let columns = feature_names(&base);
    if columns.is_empty() {
        return crate::tool_error::refuse(
            "no_feature_columns",
            "the strategy reads no feature column, so there is nothing to perturb",
            "run this on a strategy whose definition reads at least one feature column",
        );
    }
    let mut probes = Vec::new();
    for c in columns {
        let shifted = match preview(body(json!([
            {"mode": "shift_column", "column": c, "shift_bars": shift}
        ])))
        .await
        {
            Ok(v) => v,
            Err(e) => return e,
        };
        let masked = match preview(body(json!([{"mode": "mask_column", "column": c}]))).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        // A route that ignored `perturbations` would answer an ordinary preview and
        // every diff would read "no divergence". The answer carries what it applied.
        let applied = |v: &Value| {
            v.get("perturbations")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        };
        if !applied(&shifted) || !applied(&masked) {
            return crate::tool_error::refuse(
                "perturbation_not_applied",
                "the platform answered without echoing the perturbation it applied. An unperturbed \
                 run diffs to no divergence, so this result is refused rather than read as clean.",
                "do not cite this as a clean result; call again, and if it repeats report that the \
                 platform is not applying perturbations",
            );
        }
        probes.push(ColumnProbe { column: c, shifted, masked });
    }
    assess_perturbations(&base, &probes, shift)
}

#[cfg(test)]
mod perturbation_tests {
    use super::*;

    fn preview(entries: u64, bars_true: i64, fires: i64) -> Value {
        json!({
            "entries": entries,
            "features": [{"name": "ema_3"}],
            "signals": [{"signal": "entry", "bars_true": bars_true, "fires": fires}],
            "sample_events": [{"signal": "entry", "bar": 60}],
        })
    }

    fn probe(column: &str, shifted: Value, masked: Value) -> ColumnProbe {
        ColumnProbe { column: column.into(), shifted, masked }
    }

    /// A decisive column that does not respond to a time shift is the seeded
    /// lookahead, and it is named.
    #[test]
    fn a_decisive_column_that_ignores_a_shift_is_named() {
        let base = preview(5, 40, 5);
        let masked = json!({"entries": 0, "never_fires_code": "perturbed_column_has_no_value",
                            "signals": [{"signal": "entry", "bars_true": 0, "fires": 0}]});
        let out = assess_perturbations(&base, &[probe("ema_3", base.clone(), masked)], 1);
        assert_eq!(out["detail"]["has_bias"], true);
        assert_eq!(out["detail"]["biased_indicators"], json!(["ema_3"]));
        assert_eq!(out["detail"]["verdict"], "suspected_lookahead");
    }

    /// A clean strategy: moving the column moves the entries.
    #[test]
    fn a_column_that_responds_to_a_shift_is_clean() {
        let base = preview(5, 40, 5);
        let shifted = preview(5, 44, 6);
        let masked = json!({"entries": 0, "never_fires_code": "perturbed_column_has_no_value",
                            "signals": [{"signal": "entry", "bars_true": 0, "fires": 0}]});
        let out = assess_perturbations(&base, &[probe("ema_3", shifted, masked)], 1);
        assert_eq!(out["detail"]["has_bias"], false);
        assert_eq!(out["detail"]["verdict"], "no_pattern_found");
    }

    /// A column masking does not move cannot be tested, and says so instead of
    /// reading as clean.
    #[test]
    fn an_indecisive_column_is_inconclusive_not_clean() {
        let base = preview(5, 40, 5);
        let out = assess_perturbations(&base, &[probe("rsi_14", base.clone(), base.clone())], 1);
        assert_eq!(out["detail"]["verdict"], "inconclusive");
        assert_eq!(out["detail"]["columns"][0]["classification"], "not_decisive");
    }

    /// No entries, no test: the negative would be about the window.
    #[test]
    fn a_baseline_that_never_fires_is_refused() {
        let base = preview(0, 0, 0);
        let out = assess_perturbations(&base, &[], 1);
        assert_eq!(out["error"]["code"], "baseline_never_fires");
    }
}
// ── Declarations ─────────────────────────────────────────────────────────────

/// One declaration per tool (AGENT-006 §2.3).
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "deflated_sharpe",
            "audit",
            Risk::Read,
            "Discounts a run's Sharpe ratio by how many strategies were tried to find it. \
             Refuses when the run is not sealed into a hypothesis family, because an unsealed \
             family has no honest denominator.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": {"type": "string"},
                    "experiment_id": {"type": "string", "description": "the family, if the run belongs to one"}
                }
            }),
        )
        .internal()
        .evidence(&["run_id", "trial_id"])
        .summary(
            "deflated Sharpe {deflated_sharpe} against a raw {sharpe} over {n_obs} observations, \
             discounted for {trials} effective trials; {verdict}",
            &[
                ("deflated_sharpe", ""),
                ("sharpe", ""),
                ("n_obs", "obs"),
                ("trials", "trials"),
                ("verdict", ""),
            ],
        )
        .notes("deflated_sharpe"),
        ToolSpec::new(
            "min_track_record",
            "audit",
            Risk::Read,
            "How long a track record has to be before this Sharpe could be distinguished from \
             luck. A blocking precondition for promotion, not an advisory number.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": { "run_id": {"type": "string"} }
            }),
        )
        .internal()
        .evidence(&["run_id"])
        .summary(
            "needs {required_years} to distinguish this Sharpe from luck; the run covers \
             {observed_years}; {verdict}",
            &[
                ("required_years", "years"),
                ("observed_years", "years"),
                ("verdict", ""),
            ],
        ),
        ToolSpec::new(
            "metric_integrity",
            "audit",
            Risk::Read,
            "Re-derives a metric from the platform's own record of a run and diffs it against              the value being reported. Run it on any number before putting it in a finding.",
            json!({
                "type": "object",
                "required": ["run_id", "metric", "claimed_value"],
                "properties": {
                    "run_id": {"type": "string"},
                    "metric": {
                        "type": "string",
                        "enum": ["cagr", "total_return", "sharpe", "sortino", "calmar",
                                 "detrended_sharpe", "max_drawdown", "profit_factor", "expectancy"]
                    },
                    "claimed_value": {"type": "number", "description": "the number about to be reported"}
                }
            }),
        )
        .internal()
        .evidence(&["run_id"])
        .summary(
            "reported {claimed_value} against a recomputed {platform_value} ({metric});              {verdict}",
            &[
                ("claimed_value", ""),
                ("platform_value", ""),
                ("metric", ""),
                ("verdict", ""),
            ],
        )
        .notes("metric_integrity"),
        ToolSpec::new(
            "probability_of_overfit",
            "audit",
            Risk::Read,
            "How often this experiment's in-sample-best configuration underperformed out of              sample, by combinatorially-symmetric cross-validation. Refuses below ten distinct              configurations, where the statistic has too few rankings to take a value.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": {
                    "experiment_id": {"type": "string"},
                    "n_periods": {"type": "integer", "minimum": 2, "maximum": 16, "default": 8}
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .summary(
            "PBO {probability_of_overfit} over {n_configs} configurations and {n_periods}; {verdict}",
            &[
                ("probability_of_overfit", ""),
                ("n_configs", "configs"),
                ("n_periods", "periods"),
                ("verdict", ""),
            ],
        )
        .notes("probability_of_overfit"),
        ToolSpec::new(
            "family_wise_test",
            "audit",
            Risk::Read,
            "Which candidates in this experiment survive once you account for having generated              the whole family, by Romano-Wolf stepdown over their own return series. Use it              before reporting the best result of a sweep.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": {
                    "experiment_id": {"type": "string"},
                    "alpha": {"type": "number", "minimum": 0.001, "maximum": 0.2, "default": 0.05},
                    "seed": {"type": "integer", "description": "fixed for reproducibility; defaults to 0"}
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .summary(
            "{n_survivors} of {n_candidates} survive at alpha={alpha}; {verdict}",
            &[
                ("n_survivors", ""),
                ("n_candidates", "candidates"),
                ("alpha", ""),
                ("verdict", ""),
            ],
        )
        .notes("family_wise_test"),
        ToolSpec::new(
            "future_perturbation_test",
            "audit",
            Risk::Read,
            "Re-runs a draft strategy with each feature column moved in time and masked, and \
             names the columns that decide the strategy yet do not respond to being moved. \
             Free: it uses the preview, which spends no trial.",
            json!({
                "type": "object",
                "required": ["definition", "instrument_id", "start", "end"],
                "properties": {
                    "definition": {"type": "object", "description": "the draft, as preview_signals takes it"},
                    "instrument_id": {"type": "string"},
                    "start": {"type": "string"},
                    "end": {"type": "string"},
                    "timeframe": {"type": "string", "default": "1m"},
                    "shift_bars": {"type": "integer", "minimum": 1, "maximum": 20, "default": 1}
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::None)
        .summary(
            "lookahead bias {has_bias} over {n_columns}; suspect: {biased_indicators}; {verdict}",
            &[("has_bias", ""), ("n_columns", "columns"), ("biased_indicators", ""), ("verdict", "")],
        ),
        ToolSpec::new(
            "feature_leakage_scan",
            "audit",
            Risk::Read,
            "Scans analysis or feature code you wrote for constructs that read the future: a \
             negative shift, a centred window, a backward fill, a whole-sample mean or scaler \
             used as a normaliser, a label feeding a feature. Free and local. No findings is \
             not a clean bill: follow it with future_perturbation_test.",
            json!({
                "type": "object",
                "required": ["source"],
                "properties": {
                    "source": { "type": "string", "description": "the Python source, whole cell or file", "maxLength": 200000 }
                }
            }),
        )
        .internal()
        .spends_trial(TrialKind::None)
        .summary(
            "{verdict}: {n_blocking} blocking, {n_flags} flagged over {lines_scanned}",
            &[("verdict", ""), ("n_blocking", "findings"), ("n_flags", "findings"), ("lines_scanned", "lines")],
        ),
        ToolSpec::new(
            "check_holdout_budget",
            "audit",
            Risk::Read,
            "Whether this experiment's sealed holdout has been spent, and what was recorded \
             when. Ask this before planning work that depends on a holdout look.",
            json!({
                "type": "object",
                "required": ["experiment_id"],
                "properties": { "experiment_id": {"type": "string"} }
            }),
        )
        .internal()
        .summary(
            "holdout {state}; {looks_taken} of {looks_allowed} used; {can_run_note}",
            &[
                ("state", ""),
                ("looks_taken", "looks"),
                ("looks_allowed", "looks"),
                ("can_run_note", ""),
            ],
        ),
        ToolSpec::new(
            "spend_holdout_look",
            "audit",
            Risk::Read,
            "Evaluates the candidate against the sealed holdout. This is the only tool that \
             can read holdout data, it can be done once per experiment ever, and it needs a \
             human's approval.",
            json!({
                "type": "object",
                "required": ["experiment_id", "reason"],
                "properties": {
                    "experiment_id": {"type": "string"},
                    "reason": {
                        "type": "string",
                        "description": "why this candidate is ready — recorded permanently in the access log"
                    }
                }
            }),
        )
        .internal()
        .evidence(&["experiment_id"])
        .envelope(Envelope::SealedHoldout)
        // Bounded independently of the envelope. Approval is a human saying yes
        // once; this is the arithmetic refusing a second attempt inside one run
        // even if a human would have said yes again.
        .max_calls_per_run(1)
        .spends_trial(TrialKind::None)
        .summary(
            "holdout evaluated: {verdict}; the experiment is now {state} and this door is \
             closed permanently",
            &[("verdict", ""), ("state", "")],
        )
        .notes("spend_holdout_look"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tier's whole claim in one assertion.
    #[test]
    fn spending_the_holdout_needs_approval_and_cannot_be_repeated() {
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "spend_holdout_look")
            .expect("declared");
        assert_eq!(spec.envelope, Envelope::SealedHoldout);
        assert!(spec.envelope.requires_approval());
        assert_eq!(spec.max_calls_per_run, Some(1));
    }

    /// Asking what is left must never be the thing that spends it.
    #[test]
    fn checking_the_budget_is_free() {
        let spec = specs()
            .into_iter()
            .find(|s| s.name == "check_holdout_budget")
            .expect("declared");
        assert_eq!(spec.envelope, Envelope::None);
        assert_eq!(spec.spends_trial, TrialKind::None);
        assert_eq!(spec.risk, Risk::Read);
    }

    /// `spend_holdout_look` is the only tool in the whole catalogue that may
    /// reach holdout rows, and that is asserted against the catalogue rather
    /// than against this module — a second door added elsewhere is exactly the
    /// failure this is for.
    #[test]
    fn the_holdout_has_exactly_one_door() {
        let doors: Vec<&str> = crate::spec::catalog()
            .iter()
            .filter(|s| s.envelope == Envelope::SealedHoldout)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            doors,
            vec!["spend_holdout_look"],
            "another tool carries the SealedHoldout envelope. If that is deliberate it needs \
             an ADR; a holdout with two doors has been looked at twice."
        );
    }

    #[test]
    fn the_leakage_scan_finds_a_seeded_leak_names_its_line_and_never_says_clean() {
        let leaky = feature_leakage_scan(&json!({
            "source": "import pandas as pd\nz = (close - close.mean()) / close.std()\n"
        }));
        assert_eq!(leaky["summary_fields"]["verdict"], "blocking_findings");
        assert_eq!(leaky["detail"]["findings"][0]["line"], 2);
        assert_eq!(leaky["detail"]["findings"][0]["rule"], "full_window_statistic");
        let quiet = feature_leakage_scan(&json!({"source": "z = close.rolling(50).mean()\n"}));
        assert_eq!(quiet["summary_fields"]["verdict"], "no_findings_not_a_clean_bill");
        assert!(quiet["detail"]["limits"].as_str().unwrap().contains("no findings is not no leak"));
        assert_eq!(feature_leakage_scan(&json!({"source": "  "}))["error"]["code"], "empty_source");
        assert_eq!(feature_leakage_scan(&json!({}))["error"]["code"], "missing_field");
    }
}
