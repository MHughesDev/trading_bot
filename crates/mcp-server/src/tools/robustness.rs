//! `run_robustness_suite` (Set Q Q-1.8): the six checks a result has to survive, in one call.
//!
//! The six are the cost ladder, a one-bar order delay, sub-periods, the parameter
//! neighbourhood, perturbed paths and a regime split. Each is **one existing study** posted
//! to `/api/backtest/experiments/{id}/studies`, so each member run is a counted look carrying
//! the experiment id, and the trial counter rises by exactly the runs the studies report
//! (`runs_counted`). Nothing here dispatches compute of its own.
//!
//! # What a member passing means
//!
//! Defined here, once, so the reading is not the caller's:
//!
//! * `cost_ladder`: the **most pessimistic** rung's metric is positive (the ladder runs from
//!   optimistic to pessimistic);
//! * `one_bar_delay`: the metric with every order one bar late is positive;
//! * `sub_periods`, `neighbourhood`, `perturbed_paths`, `regimes`: the study's **median** is
//!   positive.
//!
//! A member whose inputs were not supplied is `not_run`, and the suite is `complete` only when
//! all six ran. The result lists which failed and which did not run; it never reports a pass
//! for a member it did not run. `perturbed_paths` uses a `synthetic_paths` study (ADR-Q-09).

use serde_json::{json, Value};

use crate::ApiClient;

const METRICS: &[&str] = &[
    "sharpe",
    "sortino",
    "calmar",
    "cagr",
    "total_return",
    "detrended_sharpe",
    "expectancy",
];

fn text(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// One planned member: its name, the study body, and how its distribution is read.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub name: &'static str,
    pub body: Value,
    pub reading: Reading,
}

/// Which number of a study's distribution decides the member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Median,
    LastMember,
    SecondMember,
}

/// The members the call asked for, plus the names of those it did not.
///
/// # Errors
/// A malformed input, as a tool error.
pub fn plan(params: &Value, metric: &str) -> Result<(Vec<Member>, Vec<&'static str>), Value> {
    let mut members = Vec::new();
    let mut not_run = Vec::new();
    let bad = |field: &str, why: &str| {
        crate::tool_error::refuse(
            "invalid_member",
            format!("`{field}` is malformed: {why}"),
            format!("fix `{field}` or omit it; an omitted member is reported as not_run"),
        )
    };

    match params.get("cost_model_refs").and_then(Value::as_array) {
        Some(a) if a.len() >= 2 && a.iter().all(Value::is_string) => members.push(Member {
            name: "cost_ladder",
            body: json!({"study_id": "robust-cost-ladder", "kind": "cost_sweep",
                "vary": {"vary": "cost_ladder", "cost_model_refs": a},
                "metric": metric, "question": "Does the edge survive the pessimistic end of the cost ladder?"}),
            reading: Reading::LastMember,
        }),
        Some(_) => return Err(bad("cost_model_refs", "needs at least two refs, optimistic first")),
        None => not_run.push("cost_ladder"),
    }

    let delay = params
        .get("max_delay_bars")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if delay != 1 {
        return Err(bad(
            "max_delay_bars",
            "this suite reads exactly one bar of delay; use timing_jitter_sensitivity for more",
        ));
    }
    members.push(Member {
        name: "one_bar_delay",
        body: json!({"study_id": "robust-delay-1", "kind": "timing_jitter",
            "vary": {"vary": "fill_delays", "bars": [0, 1]},
            "metric": metric, "question": "Does the edge survive every order reaching the venue one bar late?"}),
        reading: Reading::SecondMember,
    });

    match params.get("sub_periods").and_then(Value::as_array) {
        Some(a) if a.len() >= 2 => {
            let windows: Option<Vec<Value>> = a
                .iter()
                .map(|w| Some(json!([w.get("start")?.as_str()?, w.get("end")?.as_str()?])))
                .collect();
            let Some(windows) = windows else {
                return Err(bad("sub_periods", "each needs a start and an end"));
            };
            members.push(Member {
                name: "sub_periods",
                body: json!({"study_id": "robust-sub-periods", "kind": "walk_forward",
                    "vary": {"vary": "data_windows", "windows": windows},
                    "metric": metric, "question": "Is the edge present in each sub-period, not only in total?"}),
                reading: Reading::Median,
            });
        }
        Some(_) => return Err(bad("sub_periods", "needs at least two windows")),
        None => not_run.push("sub_periods"),
    }

    match params.get("neighbourhood") {
        Some(n) => {
            let (Some(param), Some(center), Some(step), Some(k)) = (
                n.get("param").and_then(Value::as_str),
                n.get("center").and_then(Value::as_f64),
                n.get("step").and_then(Value::as_f64),
                n.get("k").and_then(Value::as_u64),
            ) else {
                return Err(bad("neighbourhood", "needs param, center, step and k"));
            };
            members.push(Member {
                name: "neighbourhood",
                body: json!({"study_id": "robust-neighbourhood", "kind": "neighborhood",
                    "vary": {"vary": "neighborhood", "param": param, "center": center, "step": step, "k": k},
                    "metric": metric, "question": "Is the chosen parameter on a plateau rather than a spike?"}),
                reading: Reading::Median,
            });
        }
        None => not_run.push("neighbourhood"),
    }

    match params.get("perturbed_paths").and_then(Value::as_u64) {
        Some(n) if (4..=200).contains(&n) => members.push(Member {
            name: "perturbed_paths",
            body: json!({"study_id": "robust-perturbed-paths", "kind": "synthetic_paths",
                "vary": {"vary": "seeds", "n": n},
                "metric": metric, "question": "Does the edge survive resampled price paths?"}),
            reading: Reading::Median,
        }),
        Some(_) => return Err(bad("perturbed_paths", "is 4 to 200 paths")),
        None => not_run.push("perturbed_paths"),
    }

    match params.get("regimes").and_then(Value::as_array) {
        Some(a) if a.len() >= 2 => {
            let windows: Option<Vec<Value>> = a
                .iter()
                .map(|w| {
                    Some(json!([
                        w.get("start")?.as_str()?,
                        w.get("end")?.as_str()?,
                        w.get("label")?.as_str()?
                    ]))
                })
                .collect();
            let Some(windows) = windows else {
                return Err(bad("regimes", "each needs start, end and label"));
            };
            members.push(Member {
                name: "regimes",
                body: json!({"study_id": "robust-regimes", "kind": "regime_conditional",
                    "vary": {"vary": "regimes", "windows": windows},
                    "metric": metric, "question": "Does the edge hold across regimes, not in one of them?"}),
                reading: Reading::Median,
            });
        }
        Some(_) => return Err(bad("regimes", "needs at least two labelled windows")),
        None => not_run.push("regimes"),
    }
    Ok((members, not_run))
}

/// A member's verdict from its study view: `(passed, number_read, runs)`.
#[must_use]
pub fn read_member(reading: Reading, view: &Value) -> (Option<bool>, Option<f64>, usize) {
    let dist: Vec<f64> = view
        .pointer("/distribution/dist")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let runs = dist.len();
    let number = match reading {
        Reading::Median => view.pointer("/distribution/median").and_then(Value::as_f64),
        Reading::LastMember => dist.last().copied(),
        Reading::SecondMember => dist.get(1).copied(),
    };
    (number.map(|x| x > 0.0), number, runs)
}

/// Fold the member readings into the suite result.
#[must_use]
pub fn summarise(
    results: &[(&'static str, Option<bool>, Option<f64>, usize)],
    not_run: &[&'static str],
) -> Value {
    let failed: Vec<&str> = results
        .iter()
        .filter(|r| r.1 == Some(false))
        .map(|r| r.0)
        .collect();
    let unread: Vec<&str> = results
        .iter()
        .filter(|r| r.1.is_none())
        .map(|r| r.0)
        .collect();
    let runs: usize = results.iter().map(|r| r.3).sum();
    json!({
        "failed": failed,
        "not_run": not_run,
        "unreadable": unread,
        "complete": not_run.is_empty() && unread.is_empty(),
        "passed_all_that_ran": failed.is_empty() && unread.is_empty(),
        "runs_counted": runs,
        "members": results.iter().map(|r| json!({"member": r.0, "passed": r.1, "read": r.2, "runs": r.3})).collect::<Vec<_>>(),
    })
}

/// `run_robustness_suite`.
pub async fn run_robustness_suite(api: &ApiClient, params: &Value) -> Value {
    let Some(experiment_id) = text(params, "experiment_id") else {
        return crate::tool_error::missing_field("experiment_id");
    };
    let metric = text(params, "metric").unwrap_or_else(|| "sharpe".into());
    if !METRICS.contains(&metric.as_str()) {
        return crate::tool_error::refuse(
            "unsupported_metric",
            format!("metric `{metric}` is not one this tool computes."),
            format!("set metric to one of {}", METRICS.join(", ")),
        );
    }
    let (members, not_run) = match plan(params, &metric) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let path = format!(
        "/api/backtest/experiments/{}/studies",
        crate::tools::market::urlencode(&experiment_id)
    );
    let mut results = Vec::new();
    let mut views = serde_json::Map::new();
    // A study id is unique within an experiment, so a second suite on the same experiment
    // needs its own; the suffix is the platform clock, never the caller's.
    let stamp = chrono::Utc::now().timestamp();
    for m in &members {
        let mut body = m.body.clone();
        body["study_id"] = json!(format!(
            "{}-{stamp}",
            m.body["study_id"].as_str().unwrap_or("robust")
        ));
        match api.post(&path, body).await {
            Ok(view) => {
                let (passed, number, runs) = read_member(m.reading, &view);
                results.push((m.name, passed, number, runs));
                views.insert(m.name.to_string(), view);
            }
            Err(e) => {
                // A member the platform refused is reported, not skipped: the suite says which
                // of the six it could not run and why, and keeps the looks it already spent.
                results.push((m.name, None, None, 0));
                views.insert(m.name.to_string(), e.to_tool_error());
            }
        }
    }
    let summary = summarise(&results, &not_run);
    json!({
        "summary_fields": {
            "failed": summary["failed"],
            "not_run": summary["not_run"],
            "runs_counted": summary["runs_counted"],
            "complete": summary["complete"],
        },
        "detail": { "suite": summary, "studies": Value::Object(views), "metric": metric },
        "n": summary["runs_counted"],
        "ids": { "experiment_id": experiment_id },
    })
}

#[must_use]
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{ToolSpec, TrialKind};
    use crate::taxonomy::Risk;
    vec![ToolSpec::new(
        "run_robustness_suite",
        "falsification",
        Risk::Write,
        "RUN THE SIX ROBUSTNESS CHECKS on an experiment: cost ladder, one-bar order delay, \
         sub-periods, parameter neighbourhood, perturbed paths, regimes. Each is a counted study; \
         the result lists which failed and which were not run, and is complete only when all six \
         ran.",
        json!({
            "type": "object",
            "required": ["experiment_id"],
            "properties": {
                "experiment_id": { "type": "string" },
                "metric": { "type": "string" },
                "cost_model_refs": { "type": "array", "items": { "type": "string" } },
                "sub_periods": { "type": "array", "items": { "type": "object", "required": ["start", "end"], "properties": { "start": { "type": "string" }, "end": { "type": "string" } } } },
                "neighbourhood": { "type": "object", "required": ["param", "center", "step", "k"], "properties": { "param": { "type": "string" }, "center": { "type": "number" }, "step": { "type": "number" }, "k": { "type": "integer" } } },
                "perturbed_paths": { "type": "integer" },
                "regimes": { "type": "array", "items": { "type": "object", "required": ["start", "end", "label"], "properties": { "start": { "type": "string" }, "end": { "type": "string" }, "label": { "type": "string" } } } }
            }
        }),
    )
    .internal()
    .spends_trial(TrialKind::Falsification)
    .evidence(&["experiment_id"])
    .max_calls_per_run(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Value {
        json!({
            "experiment_id": "e", "cost_model_refs": ["cost:a", "cost:b"],
            "sub_periods": [{"start": "2025-01-01T00:00:00Z", "end": "2025-02-01T00:00:00Z"}, {"start": "2025-02-01T00:00:00Z", "end": "2025-03-01T00:00:00Z"}],
            "neighbourhood": {"param": "lb", "center": 20.0, "step": 2.0, "k": 3},
            "perturbed_paths": 20,
            "regimes": [{"start": "2025-01-01T00:00:00Z", "end": "2025-02-01T00:00:00Z", "label": "calm"}, {"start": "2025-02-01T00:00:00Z", "end": "2025-03-01T00:00:00Z", "label": "wild"}],
        })
    }

    #[test]
    fn all_six_members_are_planned_when_all_six_are_asked_for_and_each_is_a_known_study() {
        let (m, not_run) = plan(&full(), "sharpe").unwrap();
        assert_eq!(
            m.iter().map(|x| x.name).collect::<Vec<_>>(),
            vec![
                "cost_ladder",
                "one_bar_delay",
                "sub_periods",
                "neighbourhood",
                "perturbed_paths",
                "regimes"
            ]
        );
        assert!(not_run.is_empty());
        let kinds: Vec<&str> = m.iter().map(|x| x.body["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            vec![
                "cost_sweep",
                "timing_jitter",
                "walk_forward",
                "neighborhood",
                "synthetic_paths",
                "regime_conditional"
            ]
        );
    }

    #[test]
    fn a_one_bar_delay_study_runs_the_baseline_and_one_bar_so_a_delay_changes_the_fills() {
        let (m, _) = plan(&json!({"experiment_id": "e"}), "sharpe").unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(
            m[0].body["vary"]["bars"],
            json!([0, 1]),
            "the baseline is member 0 and one bar late is member 1"
        );
        let (_, not_run) = plan(&json!({"experiment_id": "e"}), "sharpe").unwrap();
        assert_eq!(
            not_run,
            vec![
                "cost_ladder",
                "sub_periods",
                "neighbourhood",
                "perturbed_paths",
                "regimes"
            ]
        );
    }

    #[test]
    fn the_suite_lists_which_failed_which_were_not_run_and_is_complete_only_when_all_ran() {
        let results = vec![
            ("cost_ladder", Some(false), Some(-0.2), 3),
            ("one_bar_delay", Some(true), Some(0.4), 2),
            ("regimes", None, None, 0),
        ];
        let s = summarise(&results, &["sub_periods"]);
        assert_eq!(s["failed"], json!(["cost_ladder"]));
        assert_eq!(s["unreadable"], json!(["regimes"]));
        assert_eq!(s["not_run"], json!(["sub_periods"]));
        assert_eq!(s["complete"], json!(false));
        assert_eq!(s["passed_all_that_ran"], json!(false));
        assert_eq!(
            s["runs_counted"],
            json!(5),
            "the counter rises by exactly the runs the studies report"
        );
        let all = summarise(
            &[
                ("a", Some(true), Some(1.0), 4),
                ("b", Some(true), Some(1.0), 2),
            ],
            &[],
        );
        assert_eq!(all["complete"], json!(true));
    }

    #[test]
    fn a_member_reads_the_number_its_definition_names() {
        let view = json!({"distribution": {"dist": [0.5, 0.3, -0.1], "median": 0.3}});
        assert_eq!(
            read_member(Reading::Median, &view),
            (Some(true), Some(0.3), 3)
        );
        assert_eq!(
            read_member(Reading::LastMember, &view),
            (Some(false), Some(-0.1), 3),
            "the pessimistic rung decides the ladder"
        );
        assert_eq!(read_member(Reading::SecondMember, &view).0, Some(true));
        assert_eq!(
            read_member(Reading::Median, &json!({})).0,
            None,
            "no distribution is unreadable, never a pass"
        );
    }

    #[test]
    fn malformed_members_are_refused_and_the_descriptor_fits_the_budget() {
        assert!(plan(
            &json!({"experiment_id": "e", "cost_model_refs": ["only-one"]}),
            "sharpe"
        )
        .is_err());
        assert!(plan(
            &json!({"experiment_id": "e", "perturbed_paths": 2}),
            "sharpe"
        )
        .is_err());
        assert!(plan(
            &json!({"experiment_id": "e", "max_delay_bars": 3}),
            "sharpe"
        )
        .is_err());
        let s = &specs()[0];
        let bytes = serde_json::to_string(
            &json!({"name": s.name, "description": s.description, "inputSchema": s.input_schema}),
        )
        .unwrap()
        .len();
        assert!(bytes <= 1_200, "{bytes}");
    }
}
