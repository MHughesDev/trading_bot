//! Layer 1 sweeps use the platform sampler (Set M M-1.6; FEAT-004 §2.4, S2; INV-2).
//!
//! A research strategy declares its parameters (`Param.int(lo, hi)`); this is the loop that
//! samples them. The agent supplies a **count**, never the points: the sampler is seeded, the
//! seed is recorded, and what comes back is a *sealed* report - a distribution of the members'
//! metric - with no member list, no ranking and no argmax. That is INV-2 in code: per-member
//! metrics are locals of [`run_sweep`], and [`SweepReport`] has no field that can hold the best
//! one (`the_report_has_no_member_level_field` pins the field list).
//!
//! Each member is one evaluation through the backend, which on the platform is a counted ledger
//! trial, so a sweep of N is exactly N trials, and `n_run` is what the report says it spent.
//! A member that fails to run is counted in `n_failed` and never retried: a retry would be a
//! second look at the same point.
//!
//! The sampler is the seeded uniform one (ADR-M-10). The TPE sampler in [`crate::sampler`] is
//! built over a `SearchSpace` made from a stored v1 definition and has nothing to be built from
//! here; a Layer 1 function's space is its declared params and nothing else.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::sampler::Rng;

/// The most members one sweep may run.
pub const MAX_RUNS: u32 = 200;

/// A declared parameter, as the harness reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeclaredParam {
    pub name: String,
    /// `int` or `float`.
    pub kind: String,
    pub low: f64,
    pub high: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum SweepError {
    #[error("max_runs must be 1..={MAX_RUNS}, got {0}")]
    BadRunCount(u32),
    #[error("the function declares no parameters, so there is nothing to sample")]
    NoParams,
    #[error("parameter `{0}` has an empty or non-finite range")]
    BadRange(String),
}

/// What one evaluated member reports back to the sweep.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// The metric the distribution is taken over. `None` is a run that produced none.
    pub metric: Option<f64>,
    /// Gate 0 rejected the run (a failed or missing truncation verdict).
    pub rejected: bool,
}

/// Runs one member. On the platform: compute the series, register the trial, evaluate.
pub trait Layer1Backend {
    /// # Errors
    /// Why the member could not run at all.
    fn run_member(&mut self, params: &BTreeMap<String, Value>) -> Result<Member, String>;
}

/// The sealed product. There is deliberately no field holding a member's params or metric.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SweepReport {
    pub sampler: String,
    /// Recorded so the same sweep can be reproduced: the seed fixes every point.
    pub seed: u64,
    pub n_requested: u32,
    /// Members evaluated, which is the number of trials spent.
    pub n_run: u32,
    pub n_failed: u32,
    pub n_rejected_gate0: u32,
    pub declared: Vec<DeclaredParam>,
    pub n_measured: u32,
    pub n_positive: u32,
    pub median: Option<f64>,
    pub p05: Option<f64>,
    pub p25: Option<f64>,
    pub p75: Option<f64>,
    pub p95: Option<f64>,
}

/// Decode a unit-cube coordinate into a declared range.
#[must_use]
pub fn decode(p: &DeclaredParam, u: f64) -> Value {
    let u = u.clamp(0.0, 1.0 - f64::EPSILON);
    if p.kind == "int" {
        // `low..=high` inclusive: width + 1 buckets, so both ends are reachable.
        let lo = p.low.ceil();
        let hi = p.high.floor();
        #[allow(clippy::cast_possible_truncation)]
        let v = (lo + (u * (hi - lo + 1.0)).floor()).min(hi) as i64;
        json!(v)
    } else {
        json!(p.low + u * (p.high - p.low))
    }
}

/// The points a seed draws, in order. Exposed so a test (and a reproduction) can see them; the
/// sweep report never does.
///
/// # Errors
/// An empty parameter list or a degenerate range.
pub fn draw_points(
    declared: &[DeclaredParam],
    n: u32,
    seed: u64,
) -> Result<Vec<BTreeMap<String, Value>>, SweepError> {
    if !(1..=MAX_RUNS).contains(&n) {
        return Err(SweepError::BadRunCount(n));
    }
    if declared.is_empty() {
        return Err(SweepError::NoParams);
    }
    for p in declared {
        if !(p.low.is_finite() && p.high.is_finite() && p.high > p.low) {
            return Err(SweepError::BadRange(p.name.clone()));
        }
    }
    let mut rng = Rng::new(seed);
    Ok((0..n)
        .map(|_| declared.iter().map(|p| (p.name.clone(), decode(p, rng.next_f64()))).collect())
        .collect())
}

fn quantile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted.get(i).copied()
}

/// Run the sweep.
///
/// # Errors
/// An invalid run count or parameter list. A member failing is not an error: it is counted.
pub fn run_sweep<B: Layer1Backend>(
    backend: &mut B,
    declared: Vec<DeclaredParam>,
    n: u32,
    seed: u64,
) -> Result<SweepReport, SweepError> {
    let points = draw_points(&declared, n, seed)?;
    let outcomes = points.iter().map(|p| backend.run_member(p)).collect();
    Ok(report_from(declared, n, seed, outcomes))
}

/// Fold member outcomes into the sealed report. The async platform worker evaluates members
/// itself (a trial each) and hands the outcomes here, so the report has one definition.
#[must_use]
pub fn report_from(declared: Vec<DeclaredParam>, n: u32, seed: u64, outcomes: Vec<Result<Member, String>>) -> SweepReport {
    let (mut metrics, mut failed, mut rejected, mut run) = (Vec::new(), 0_u32, 0_u32, 0_u32);
    for o in outcomes {
        match o {
            Ok(m) => {
                run += 1;
                if m.rejected {
                    rejected += 1;
                }
                if let Some(x) = m.metric.filter(|x| x.is_finite()) {
                    metrics.push(x);
                }
            }
            Err(_) => failed += 1,
        }
    }
    metrics.sort_by(f64::total_cmp);
    #[allow(clippy::cast_possible_truncation)]
    let n_measured = metrics.len() as u32;
    #[allow(clippy::cast_possible_truncation)]
    let n_positive = metrics.iter().filter(|x| **x > 0.0).count() as u32;
    SweepReport {
        sampler: "random".into(),
        seed,
        n_requested: n,
        n_run: run,
        n_failed: failed,
        n_rejected_gate0: rejected,
        declared,
        n_measured,
        n_positive,
        median: quantile(&metrics, 0.5),
        p05: quantile(&metrics, 0.05),
        p25: quantile(&metrics, 0.25),
        p75: quantile(&metrics, 0.75),
        p95: quantile(&metrics, 0.95),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> Vec<DeclaredParam> {
        vec![
            DeclaredParam { name: "lb".into(), kind: "int".into(), low: 2.0, high: 50.0 },
            DeclaredParam { name: "k".into(), kind: "float".into(), low: 0.5, high: 3.0 },
        ]
    }

    /// Counts calls and scores a member by its params.
    struct Counting {
        calls: u32,
        seen: Vec<BTreeMap<String, Value>>,
        fail_every: u32,
    }

    impl Layer1Backend for Counting {
        fn run_member(&mut self, p: &BTreeMap<String, Value>) -> Result<Member, String> {
            self.calls += 1;
            self.seen.push(p.clone());
            if self.fail_every > 0 && self.calls % self.fail_every == 0 {
                return Err("boom".into());
            }
            let lb = p["lb"].as_f64().unwrap();
            Ok(Member { metric: Some(lb - 20.0), rejected: lb as u32 % 10 == 0 })
        }
    }

    fn counting() -> Counting {
        Counting { calls: 0, seen: vec![], fail_every: 0 }
    }

    #[test]
    fn a_sweep_of_n_runs_exactly_n_members_so_it_adds_exactly_n_to_the_trial_counter() {
        let mut b = counting();
        let r = run_sweep(&mut b, declared(), 25, 7).unwrap();
        assert_eq!(b.calls, 25);
        assert_eq!((r.n_requested, r.n_run, r.n_failed), (25, 25, 0));
    }

    #[test]
    fn a_failed_member_is_counted_and_never_retried() {
        let mut b = Counting { fail_every: 5, ..counting() };
        let r = run_sweep(&mut b, declared(), 20, 7).unwrap();
        assert_eq!(b.calls, 20, "no retry: a retry would be a second look at the point");
        assert_eq!((r.n_run, r.n_failed), (16, 4));
    }

    #[test]
    fn the_seed_fixes_every_point_and_is_recorded() {
        let a = draw_points(&declared(), 30, 99).unwrap();
        assert_eq!(a, draw_points(&declared(), 30, 99).unwrap());
        assert_ne!(a, draw_points(&declared(), 30, 100).unwrap());
        let r = run_sweep(&mut counting(), declared(), 10, 99).unwrap();
        assert_eq!(r.seed, 99);
        assert_eq!(r.sampler, "random");
    }

    #[test]
    fn every_point_is_inside_its_declared_range_and_an_int_param_reaches_both_ends() {
        let pts = draw_points(&declared(), 200, 3).unwrap();
        for p in &pts {
            let lb = p["lb"].as_i64().unwrap();
            let k = p["k"].as_f64().unwrap();
            assert!((2..=50).contains(&lb) && (0.5..3.0).contains(&k), "{p:?}");
        }
        let lbs: Vec<i64> = pts.iter().map(|p| p["lb"].as_i64().unwrap()).collect();
        assert!(lbs.contains(&2) && lbs.contains(&50), "inclusive ends are reachable");
    }

    /// INV-2: no tool result contains the best member. The report type has no field that could.
    #[test]
    fn the_report_has_no_member_level_field_and_serialises_without_any_params_or_best() {
        let r = run_sweep(&mut counting(), declared(), 30, 1).unwrap();
        let v = serde_json::to_value(&r).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        for banned in ["best", "best_params", "argmax", "max", "members", "ranked", "top", "winner", "min"] {
            assert!(!keys.contains(&banned), "{banned} must not be a report field");
        }
        assert_eq!(
            keys,
            vec![
                "declared", "median", "n_failed", "n_measured", "n_positive", "n_rejected_gate0",
                "n_requested", "n_run", "p05", "p25", "p75", "p95", "sampler", "seed"
            ]
        );
        assert!(r.p95.unwrap() >= r.median.unwrap() && r.median.unwrap() >= r.p05.unwrap());
    }

    #[test]
    fn gate_0_rejections_are_counted_and_degenerate_inputs_are_refused() {
        let r = run_sweep(&mut counting(), declared(), 60, 5).unwrap();
        assert!(r.n_rejected_gate0 > 0, "lb divisible by 10 is rejected by the fake backend");
        assert_eq!(run_sweep(&mut counting(), declared(), 0, 1).unwrap_err(), SweepError::BadRunCount(0));
        assert_eq!(run_sweep(&mut counting(), declared(), 201, 1).unwrap_err(), SweepError::BadRunCount(201));
        assert_eq!(run_sweep(&mut counting(), vec![], 5, 1).unwrap_err(), SweepError::NoParams);
        let bad = vec![DeclaredParam { name: "x".into(), kind: "float".into(), low: 1.0, high: 1.0 }];
        assert_eq!(run_sweep(&mut counting(), bad, 5, 1).unwrap_err(), SweepError::BadRange("x".into()));
    }
}
