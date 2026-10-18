//! `compare_candidates` — SPEC §11.5 as one door (AGENT-013 §8.3; Set S task
//! S-10.10).
//!
//! # What this reaches, and what it replaces
//!
//! `research::compare::compare` sweeps every pair of candidates through
//! `ComparisonPlan::judge` and applies Holm across them. Until this module its
//! only production caller was the campaign's COMPARE worker, which judges one
//! candidate against one incumbent and **pins the cost axis** (ADR-C-02). The
//! tool an agent could actually call was `compare_backtests`, which puts headline
//! metrics side by side and lets a reader form an impression.
//!
//! This is the door for N candidates: it measures every candidate on the *same*
//! replicate grid, hands the paired measurements to `research::compare`, and
//! returns the matrix, the ranking and the evidence.
//!
//! # The replicate grid is the whole nuisance product
//!
//! §11.5's nuisance sources are window × cost perturbation × fill seed.
//! [`backtest::stats::compare::replicate_grid`] enumerates the grid; replicate `i`
//! sits on window `i / 3` at cost perturbation `-20 % / modelled / +20 %` by
//! `i % 3`. The cost axis is real: each cost scale is a study whose Runs pay
//! `RunConfig::cost_scale_bp` more or less than the modelled cost
//! (`RunStudySpec::cost_scale_bp`), applied where the simulator builds its fee
//! schedule. The campaign worker's note that "the cost axis is a one-line
//! extension once the cost models exist" is therefore closed for this door; the
//! campaign worker itself still pins it, and says so in its record.
//!
//! # What the caller does not choose
//!
//! There is no seed, no `delta_practical`, no ROPE bound and no replicate count
//! in the request. The seed is derived from the experiment and the candidates'
//! identity (ADR-P2-02); `delta_practical` is read from the Experiment, where it
//! was declared before any trial ran ([`backtest::suite::SuiteManager::
//! experiment_delta_practical`]) — a restated delta is a delta chosen after the
//! data; the ROPE half-width is the protocol's multiple of that delta.
//!
//! # A cost axis that did not move is said so
//!
//! A cost scale only matters if the executor honours it. The v1 simulator does;
//! an executor that ignores `cost_scale_bp` would return the same number at all
//! three scales and the comparison would be optimistic about separation without
//! saying so. [`cost_axis_inert`] checks the measurements themselves and the
//! response carries `cost_axis_inert` per candidate. It is a statement, not a
//! refusal: a strategy that never trades is legitimately cost-insensitive.

#![allow(clippy::result_large_err)]

use std::sync::Arc;

use backtest::run::{MetricKind, ParamMap};
use backtest::stats::compare::{
    replicate_grid, ComparisonFlags, ComparisonPlan, Measurement, Replicate, RopeScale,
    DEFAULT_ALPHA, MIN_REPLICATES, TARGET_REPLICATES,
};
use chrono::{DateTime, Utc};
use research::compare::{compare, Candidate, Ranking};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::campaign_driver::compare_worker::{replicate_windows, DEFAULT_SCALE_MULTIPLE};
use crate::campaign_driver::study_worker::{CampaignEvaluator, ReplicateCall, Window};

/// The cost perturbations of the grid, in hundredths of a percent. The same three
/// [`replicate_grid`] enumerates; kept here so the mapping from a replicate to the
/// study that measures it is visible next to the code that uses it.
pub const COST_SCALES_BP: [i32; 3] = [-2000, 0, 2000];

/// One candidate the caller names: a label and the parameters it runs under.
#[derive(Debug, Clone, Deserialize)]
pub struct CandidateSpec {
    pub name: String,
    #[serde(default)]
    pub params: ParamMap,
}

/// The request. **No seed, no delta, no ROPE bound, no replicate count** — see
/// the module header; a static test greps this file's request type for each.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareBody {
    /// The Experiment whose research slice the comparison is measured on.
    pub experiment: Uuid,
    pub candidates: Vec<CandidateSpec>,
}

/// Why a comparison could not be made, in a shape the route can map.
#[derive(Debug)]
pub enum CompareFail {
    /// The caller's request is not one a comparison can be made from.
    Request(String),
    /// The Experiment is not this user's, or does not exist.
    NotFound,
    /// The experiment declares no objective, so there is no metric to compare on.
    NoObjective,
    /// Measuring failed for a reason that is not the caller's.
    Measure(String),
    /// The protocol refused (too few paired replicates, a difference outside the
    /// declared scale). A refusal is not recorded as a verdict.
    Refused(String),
}

/// A platform-held seed derived from the experiment and the candidates' identity,
/// so the same comparison draws the same grid and a caller has no argument
/// through which a friendlier one could be chosen.
#[must_use]
pub fn grid_seed(experiment: Uuid, candidates: &[CandidateSpec]) -> u64 {
    let mut h = Sha256::new();
    h.update(experiment.as_bytes());
    h.update(b"\x1fcompare-candidates\x1f");
    // Sorted so the order the caller listed them in does not move the grid.
    let mut ids: Vec<String> = candidates
        .iter()
        .map(|c| {
            format!(
                "{}\x1e{}",
                c.name,
                serde_json::to_string(&c.params).unwrap_or_default()
            )
        })
        .collect();
    ids.sort();
    for id in ids {
        h.update(id.as_bytes());
        h.update(b"\x1f");
    }
    let d = h.finalize();
    u64::from_be_bytes(d[..8].try_into().expect("sha256 is 32 bytes"))
}

/// For each cost scale in [`COST_SCALES_BP`], the replicate indices that sit on
/// it and the window index each of them uses.
#[must_use]
pub fn cells(grid: &[Replicate]) -> Vec<(i32, Vec<(usize, u32)>)> {
    COST_SCALES_BP
        .iter()
        .map(|&scale| {
            (
                scale,
                grid.iter()
                    .enumerate()
                    .filter(|(_, r)| r.cost_bp == scale)
                    .map(|(i, r)| (i, r.window))
                    .collect(),
            )
        })
        .collect()
}

/// Line the per-candidate values up on the grid and keep only replicates **every**
/// candidate has a value for. A replicate where any candidate failed carries no
/// paired difference for that candidate, and imputing one would be inventing a
/// replicate; the count dropped is returned so the response can say.
#[must_use]
pub fn align(
    grid: &[Replicate],
    per_candidate: &[Vec<Option<f64>>],
) -> (Vec<Vec<Measurement>>, usize) {
    let mut out: Vec<Vec<Measurement>> = vec![Vec::new(); per_candidate.len()];
    let mut dropped = 0;
    for (i, replicate) in grid.iter().enumerate() {
        let values: Vec<Option<f64>> = per_candidate
            .iter()
            .map(|v| v.get(i).copied().flatten().filter(|x| x.is_finite()))
            .collect();
        if values.iter().all(Option::is_some) {
            for (c, v) in values.into_iter().enumerate() {
                out[c].push(Measurement {
                    replicate: *replicate,
                    value: v.expect("checked"),
                });
            }
        } else {
            dropped += 1;
        }
    }
    (out, dropped)
}

/// True when a candidate's measurements on the same window are identical across
/// every cost scale it has — the signature of an executor that ignored the
/// perturbation (or of a strategy that never trades). Needs at least two scales
/// on at least one window to say anything; otherwise false.
#[must_use]
pub fn cost_axis_inert(measurements: &[Measurement]) -> bool {
    use std::collections::BTreeMap;
    let mut by_window: BTreeMap<u32, Vec<f64>> = BTreeMap::new();
    for m in measurements {
        by_window.entry(m.replicate.window).or_default().push(m.value);
    }
    let comparable: Vec<&Vec<f64>> = by_window.values().filter(|v| v.len() >= 2).collect();
    !comparable.is_empty()
        && comparable
            .iter()
            .all(|v| v.iter().all(|x| x.to_bits() == v[0].to_bits()))
}

/// Measure every candidate on the grid. One study per (candidate, cost scale),
/// each over that scale's windows, so a replicate's value is a real Run under that
/// replicate's perturbation.
async fn measure(
    evaluator: &dyn CampaignEvaluator,
    user: Uuid,
    experiment: Uuid,
    metric: MetricKind,
    candidates: &[CandidateSpec],
    grid: &[Replicate],
    windows: &[(DateTime<Utc>, DateTime<Utc>)],
    tag: &str,
) -> Result<Vec<Vec<Option<f64>>>, CompareFail> {
    let never = || false;
    let mut out = vec![vec![None; grid.len()]; candidates.len()];
    for (c, cand) in candidates.iter().enumerate() {
        for (scale, cell) in cells(grid) {
            if cell.is_empty() {
                continue;
            }
            let ws: Vec<(DateTime<Utc>, DateTime<Utc>)> = cell
                .iter()
                .map(|(_, w)| windows[(*w as usize).min(windows.len() - 1)])
                .collect();
            let values = evaluator
                .replicate_metrics(
                    user,
                    ReplicateCall {
                        experiment,
                        study_id: format!("cmpc-{tag}-{c}-{}", scale_label(scale)),
                        params: cand.params.clone(),
                        windows: ws,
                        metric,
                        question: format!(
                            "compare_candidates: `{}` at cost {} across {} windows",
                            cand.name,
                            scale_label(scale),
                            cell.len()
                        ),
                        cost_scale_bp: scale,
                    },
                    &never,
                )
                .await
                .map_err(CompareFail::Measure)?;
            for ((i, _), v) in cell.iter().zip(values) {
                out[c][*i] = v;
            }
        }
    }
    Ok(out)
}

fn scale_label(scale: i32) -> String {
    match scale {
        0 => "modelled".to_string(),
        s if s > 0 => format!("plus{}", s / 100),
        s => format!("minus{}", -s / 100),
    }
}

/// Everything the comparison needs from outside, so the whole flow is testable
/// against a fake evaluator.
pub struct Inputs<'a> {
    pub evaluator: &'a dyn CampaignEvaluator,
    pub user: Uuid,
    pub experiment: Uuid,
    pub metric: MetricKind,
    pub research: Window,
    pub delta_practical: f64,
    pub profile_id: String,
    pub n_eff: f64,
    pub trials: i64,
}

/// Run the comparison: measure, pair, judge every pair, Holm, matrix.
///
/// # Errors
/// [`CompareFail`].
pub async fn run(inputs: Inputs<'_>, body: &CompareBody) -> Result<Value, CompareFail> {
    if body.candidates.len() < 2 {
        return Err(CompareFail::Request(format!(
            "a comparison needs at least two candidates; got {}",
            body.candidates.len()
        )));
    }
    let seed = grid_seed(inputs.experiment, &body.candidates);
    let grid = replicate_grid(TARGET_REPLICATES, seed);
    // One window per (window index) the grid names.
    let n_windows = grid
        .iter()
        .map(|r| r.window as usize)
        .max()
        .map_or(0, |m| m + 1);
    let windows = replicate_windows(&inputs.research, n_windows);
    if windows.is_empty() {
        return Err(CompareFail::Request(
            "the Experiment's research slice is empty".to_string(),
        ));
    }
    let tag = format!("{:016x}", seed);
    let measured = measure(
        inputs.evaluator,
        inputs.user,
        inputs.experiment,
        inputs.metric,
        &body.candidates,
        &grid,
        &windows,
        &tag[..8],
    )
    .await?;
    let (paired, dropped) = align(&grid, &measured);
    let achieved = paired.first().map_or(0, Vec::len);
    if achieved < MIN_REPLICATES {
        return Err(CompareFail::Refused(format!(
            "only {achieved} replicates were measured for every candidate ({dropped} dropped); \
             the protocol needs {MIN_REPLICATES}"
        )));
    }
    let scale = RopeScale::new(DEFAULT_SCALE_MULTIPLE * inputs.delta_practical)
        .map_err(|e| CompareFail::Refused(e.to_string()))?;
    let plan = ComparisonPlan {
        profile_id: inputs.profile_id.clone(),
        n_eff: inputs.n_eff,
        delta_practical: inputs.delta_practical,
        scale,
        trials_a: inputs.trials,
        trials_b: inputs.trials,
        k_max: achieved,
        flags: ComparisonFlags::default(),
        alpha: DEFAULT_ALPHA,
    };
    let candidates: Vec<Candidate> = body
        .candidates
        .iter()
        .zip(&paired)
        .map(|(spec, m)| Candidate {
            name: spec.name.clone(),
            measurements: m.clone(),
        })
        .collect();
    let sweep = compare(&candidates, &plan).map_err(|e| CompareFail::Request(e.to_string()))?;

    let (ranking_kind, order) = match &sweep.ranking {
        Ranking::Ranked { order } => ("ranked", order.clone()),
        Ranking::Indistinguishable => ("indistinguishable", Vec::new()),
    };
    let adjusted: Vec<Value> = sweep
        .adjusted_p
        .iter()
        .map(|((a, b), p)| json!({ "a": a, "b": b, "holm_adjusted_p": p }))
        .collect();
    let refused: Vec<Value> = sweep
        .refused
        .iter()
        .map(|(a, b, why)| json!({ "a": a, "b": b, "why": why }))
        .collect();
    let inert: Vec<Value> = candidates
        .iter()
        .map(|c| json!({ "name": c.name, "cost_axis_inert": cost_axis_inert(&c.measurements) }))
        .collect();
    Ok(json!({
        "ranking": ranking_kind,
        "order": order,
        "matrix": matrix_json(&sweep.matrix),
        "holm_adjusted": adjusted,
        "refused_pairs": refused,
        "replicates": {
            "achieved": achieved,
            "dropped": dropped,
            "target": TARGET_REPLICATES,
            "axes": ["window", "cost_perturbation_bp"],
            "cost_perturbations_bp": COST_SCALES_BP,
        },
        "cost_axis": inert,
        "criteria": {
            "wins": "P(A>B) >= 0.75 and P(A>B) - CI_lower > 0.5",
            "equivalence": "anytime-valid confidence sequence inside the ROPE of +/- delta_practical",
            "multiplicity": "Holm across pairs",
        },
        "delta_practical": inputs.delta_practical,
        "profile_id": inputs.profile_id,
        "n_eff": inputs.n_eff,
        "note": "a Reject means A's advantage is smaller than delta_practical, not that B is better; \
                 only a Promote establishes an edge, and `indistinguishable` is a real answer",
    }))
}

/// The Multiple Comparison Matrix as rows of cells. Built cell by cell because the
/// matrix keys its cells by `(a, b)` tuples, which JSON objects cannot hold: the
/// derived serialisation fails, and a matrix that serialises to `null` reads as
/// "no evidence" when it is the evidence.
fn matrix_json(m: &backtest::stats::compare::ComparisonMatrix) -> Value {
    let names = m.names();
    let cells: Vec<Value> = names
        .iter()
        .flat_map(|a| {
            names.iter().filter(move |b| *b != a).filter_map(move |b| {
                m.get(a, b).map(|cell| {
                    json!({ "a": a, "b": b, "cell": serde_json::to_value(cell).unwrap_or(Value::Null) })
                })
            })
        })
        .collect();
    json!({ "names": names, "cells": cells })
}

/// Shared evaluator handle, for the route.
pub type Evaluator = Arc<dyn CampaignEvaluator>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::campaign_driver::study_worker::{SweepCall, SweepOutcome};
    use crate::campaign_driver::study_worker::EvaluationSpec;
    use std::sync::Mutex;

    /// An evaluator that scores each candidate by a closure over (params, cost
    /// scale, window start), recording every call.
    struct Fake {
        score: Box<dyn Fn(&ParamMap, i32, usize) -> Option<f64> + Send + Sync>,
        calls: Mutex<Vec<(i32, usize)>>,
    }

    #[async_trait::async_trait]
    impl CampaignEvaluator for Fake {
        async fn ensure_experiment(
            &self,
            _: Uuid,
            _: Uuid,
            _: &EvaluationSpec,
            _: f64,
        ) -> Result<Uuid, String> {
            unreachable!()
        }
        async fn sweep(
            &self,
            _: Uuid,
            _: SweepCall,
            _: &(dyn Fn() -> bool + Send + Sync),
        ) -> Result<SweepOutcome, String> {
            unreachable!()
        }
        async fn replicate_metrics(
            &self,
            _: Uuid,
            call: ReplicateCall,
            _: &(dyn Fn() -> bool + Send + Sync),
        ) -> Result<Vec<Option<f64>>, String> {
            self.calls
                .lock()
                .unwrap()
                .push((call.cost_scale_bp, call.windows.len()));
            Ok((0..call.windows.len())
                .map(|w| (self.score)(&call.params, call.cost_scale_bp, w))
                .collect())
        }
    }

    fn research() -> Window {
        Window {
            start: "2024-01-01T00:00:00Z".parse().unwrap(),
            end: "2024-07-01T00:00:00Z".parse().unwrap(),
        }
    }

    fn spec(name: &str, k: i64) -> CandidateSpec {
        let mut params = ParamMap::new();
        params.insert("k".into(), json!(k));
        CandidateSpec {
            name: name.into(),
            params,
        }
    }

    fn inputs<'a>(ev: &'a dyn CampaignEvaluator) -> Inputs<'a> {
        inputs_with(ev, 0.1)
    }

    fn inputs_with<'a>(ev: &'a dyn CampaignEvaluator, delta: f64) -> Inputs<'a> {
        Inputs {
            evaluator: ev,
            user: Uuid::nil(),
            experiment: Uuid::from_u128(7),
            metric: MetricKind::Sharpe,
            research: research(),
            delta_practical: delta,
            profile_id: "strict_v1".into(),
            n_eff: 10.0,
            trials: 10,
        }
    }

    fn k_of(p: &ParamMap) -> f64 {
        p.get("k").and_then(Value::as_f64).unwrap_or(0.0)
    }

    /// Small deterministic jitter so paired differences have a spread.
    fn jitter(window: usize, scale: i32) -> f64 {
        let x = (window as f64 * 12.9898 + f64::from(scale) * 0.0043).sin() * 43758.5453;
        (x - x.floor() - 0.5) * 0.02
    }

    #[tokio::test]
    async fn two_copies_of_the_same_strategy_are_indistinguishable() {
        let ev = Fake {
            score: Box::new(|p, s, w| Some(1.0 + k_of(p) * 0.0 + jitter(w, s))),
            calls: Mutex::default(),
        };
        let body = CompareBody {
            experiment: Uuid::from_u128(7),
            candidates: vec![spec("a", 1), spec("b", 2)],
        };
        let out = run(inputs(&ev), &body).await.unwrap();
        assert_eq!(out["ranking"], "indistinguishable", "{out}");
        assert_eq!(out["replicates"]["achieved"], 29);
    }

    #[tokio::test]
    async fn a_planted_dominant_candidate_ranks_first_on_the_whole_grid() {
        let ev = Fake {
            // The confidence sequence's width follows the declared ROPE scale
            // (20 x delta), not the observed spread, so a planted edge has to be
            // large against it: delta 0.5 gives a scale of 10 and steps of 4.
            score: Box::new(|p, s, w| Some(1.0 + k_of(p) * 4.0 + jitter(w, s))),
            calls: Mutex::default(),
        };
        let body = CompareBody {
            experiment: Uuid::from_u128(7),
            candidates: vec![spec("weak", 0), spec("strong", 2), spec("mid", 1)],
        };
        let out = run(inputs_with(&ev, 0.5), &body).await.unwrap();
        assert_eq!(out["ranking"], "ranked", "{out}");
        assert_eq!(out["order"][0], "strong", "{out}");
        // The matrix is evidence, not null: every ordered pair has a cell.
        assert_eq!(out["matrix"]["cells"].as_array().unwrap().len(), 6, "{out}");
        // Holm across every pair of three is three adjusted p-values.
        assert_eq!(out["holm_adjusted"].as_array().unwrap().len(), 3);
        // The cost axis was measured at all three perturbations for every candidate.
        let calls = ev.calls.lock().unwrap();
        for scale in COST_SCALES_BP {
            assert_eq!(
                calls.iter().filter(|(s, _)| *s == scale).count(),
                3,
                "one study per candidate at {scale}"
            );
        }
    }

    #[tokio::test]
    async fn the_cost_axis_is_reported_inert_when_the_executor_ignores_it() {
        let ev = Fake {
            score: Box::new(|p, _s, w| Some(1.0 + k_of(p) * 0.5 + w as f64 * 0.01)),
            calls: Mutex::default(),
        };
        let body = CompareBody {
            experiment: Uuid::from_u128(7),
            candidates: vec![spec("a", 0), spec("b", 2)],
        };
        let out = run(inputs(&ev), &body).await.unwrap();
        assert_eq!(out["cost_axis"][0]["cost_axis_inert"], true, "{out}");
    }

    #[tokio::test]
    async fn a_failed_replicate_is_dropped_for_everyone_not_imputed() {
        let ev = Fake {
            // Candidate k=2 fails on window 0 at every cost.
            score: Box::new(|p, s, w| {
                if k_of(p) > 1.0 && w == 0 {
                    None
                } else {
                    Some(1.0 + k_of(p) * 0.5 + jitter(w, s))
                }
            }),
            calls: Mutex::default(),
        };
        let body = CompareBody {
            experiment: Uuid::from_u128(7),
            candidates: vec![spec("a", 0), spec("b", 2)],
        };
        let out = run(inputs(&ev), &body).await.unwrap();
        let dropped = out["replicates"]["dropped"].as_u64().unwrap();
        assert_eq!(dropped, 3, "window 0 holds three replicates: {out}");
        assert_eq!(out["replicates"]["achieved"], 26);
    }

    #[tokio::test]
    async fn too_few_replicates_is_a_refusal_not_a_verdict() {
        let ev = Fake {
            score: Box::new(|_, _, _| None),
            calls: Mutex::default(),
        };
        let body = CompareBody {
            experiment: Uuid::from_u128(7),
            candidates: vec![spec("a", 0), spec("b", 2)],
        };
        assert!(matches!(
            run(inputs(&ev), &body).await,
            Err(CompareFail::Refused(_))
        ));
    }

    #[tokio::test]
    async fn one_candidate_is_not_a_comparison() {
        let ev = Fake {
            score: Box::new(|_, _, _| Some(1.0)),
            calls: Mutex::default(),
        };
        let body = CompareBody {
            experiment: Uuid::from_u128(7),
            candidates: vec![spec("a", 0)],
        };
        assert!(matches!(
            run(inputs(&ev), &body).await,
            Err(CompareFail::Request(_))
        ));
    }

    #[test]
    fn the_seed_is_derived_from_identity_and_not_from_the_order_listed() {
        let ab = [spec("a", 1), spec("b", 2)];
        let ba = [spec("b", 2), spec("a", 1)];
        let other = [spec("a", 1), spec("c", 2)];
        let e = Uuid::from_u128(1);
        assert_eq!(grid_seed(e, &ab), grid_seed(e, &ba));
        assert_ne!(grid_seed(e, &ab), grid_seed(e, &other));
        assert_ne!(grid_seed(e, &ab), grid_seed(Uuid::from_u128(2), &ab));
    }

    /// The request has no field through which a caller could choose the seed, the
    /// effect size, the ROPE bound or the replicate count, and refuses one sent
    /// anyway (`deny_unknown_fields`).
    #[test]
    fn the_request_has_no_seed_delta_rope_or_replicate_field() {
        let src = include_str!("compare_candidates.rs");
        let start = src.find("pub struct CompareBody").expect("request type");
        let end = src[start..].find("\n}\n").expect("end of struct") + start;
        let body = &src[start..end];
        for forbidden in ["seed", "delta", "rope", "scale", "replicates", "alpha", "k_max"] {
            assert!(
                !body.to_lowercase().contains(forbidden),
                "the request type must not carry `{forbidden}`:\n{body}"
            );
        }
        for field in ["seed", "delta_practical", "rope_scale", "replicates"] {
            let err = serde_json::from_value::<CompareBody>(json!({
                "experiment": Uuid::nil(),
                "candidates": [],
                field: 1,
            }));
            assert!(err.is_err(), "`{field}` must be refused by name");
        }
    }

    #[test]
    fn the_grid_covers_every_cost_perturbation_on_a_shared_window() {
        let grid = replicate_grid(TARGET_REPLICATES, 3);
        let cs = cells(&grid);
        assert_eq!(cs.len(), 3);
        assert_eq!(cs.iter().map(|(_, c)| c.len()).sum::<usize>(), 29);
        assert!(cs.iter().all(|(_, c)| !c.is_empty()));
    }
}
