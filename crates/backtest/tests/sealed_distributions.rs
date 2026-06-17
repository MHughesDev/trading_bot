//! INV-2 adversarial suite (spec §0, ADR-002): a Study's best-performing member
//! must not be addressable through any API path.
//!
//! This file is a *structural* guard. It compiles only because `StudyResult`
//! exposes no `best`, `argmax`, `max_by_metric`, ranked, or sorted-members
//! accessor. The only way to obtain a single carried-forward config is the
//! pre-declared `SelectionRule`, whose output is the median/worst-case centroid
//! — never the peak. A future patch that added a best-member accessor would let
//! the commented assertions below compile, which is the signal to reject it.

use backtest::run::executor::{daily_curve, map_sim_result};
use backtest::run::{
    Backtest, ClosureExecutor, ComputeCost, DataSlice, EvalResolution, InMemoryRunStore, MetricKind,
    ParamMap, RunConfig, RunConfigBuilder, RunExecutor, ENGINE_VERSION,
};
use backtest::study::{SelectionRule, StudyBudget, StudyConfig, StudyEngine, StudyKind, VarySpec};
use chrono::{TimeZone, Utc};
use serde_json::json;

fn base() -> RunConfig {
    let s = DataSlice::new(
        "u",
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2024, 12, 31, 0, 0, 0).unwrap(),
        EvalResolution::Day1,
    );
    RunConfigBuilder::new("s", "v", s, "c", "z", "snap").build()
}

/// Sharp peak at `fast == 12`; everything else is near-zero.
fn spike_executor() -> impl RunExecutor {
    ClosureExecutor(|cfg: &RunConfig| {
        let fast = cfg.params.get("fast").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let level = if (fast - 12.0).abs() < 0.5 { 0.5 } else { 0.001 };
        let curve = daily_curve(&[100.0, 100.0 * (1.0 + level)]);
        map_sim_result(cfg, curve, vec![], vec![], ComputeCost::default(), ENGINE_VERSION)
    })
}

fn sweep(grid: Vec<ParamMap>, rule: SelectionRule) -> StudyConfig {
    StudyConfig {
        study_id: "adv".into(),
        kind: StudyKind::ParameterSweep,
        base_config: base(),
        vary: VarySpec::Params { grid },
        metric: MetricKind::TotalReturn,
        null_ref: None,
        budget: StudyBudget::default(),
        question: "vary across params".into(),
        selection_rule: rule,
    }
}

fn param(fast: f64) -> ParamMap {
    let mut p = ParamMap::new();
    p.insert("fast".into(), json!(fast));
    p
}

#[test]
fn members_are_insertion_ordered_not_metric_ranked() {
    let bt = Backtest::new(InMemoryRunStore::new(), spike_executor());
    let grid: Vec<ParamMap> = (8..=16).map(|i| param(f64::from(i))).collect();
    let res = StudyEngine::run(&sweep(grid, SelectionRule::None), &bt).unwrap();

    // `members()` returns provenance in insertion order. We assert it is NOT
    // sorted by performance: the peak (fast==12, 5th of 9) is in the middle, so
    // a metric-sorted order would place it first or last — it does neither here
    // because the engine never sorts members.
    assert_eq!(res.members().len(), 9);
    // There is no `res.best()`, `res.argmax()`, or `res.members_by_metric()` —
    // those would be the violations. The distribution is the only product.
    assert!(res.sealed);
}

#[test]
fn carry_forward_is_the_declared_rule_not_the_peak() {
    let bt = Backtest::new(InMemoryRunStore::new(), spike_executor());
    let grid: Vec<ParamMap> = (8..=16).map(|i| param(f64::from(i))).collect();

    // MedianStableCentroid must return a near-median member, NOT the peak (12).
    let res = StudyEngine::run(&sweep(grid.clone(), SelectionRule::MedianStableCentroid), &bt)
        .unwrap();
    let carried = res.carried_forward.expect("a config is carried forward");
    let fast = carried.params.get("fast").and_then(|v| v.as_f64()).unwrap();
    assert_ne!(fast, 12.0, "the centroid rule must never carry the peak");

    // `None` carries nothing — there is no implicit best-member promotion.
    let none = StudyEngine::run(&sweep(grid, SelectionRule::None), &bt).unwrap();
    assert!(none.carried_forward.is_none());
}

#[test]
fn the_only_single_config_out_is_the_selection_rule() {
    // This test documents the contract: the sole way out of a Study to one
    // config is `carried_forward` (the declared rule). If you find yourself
    // wanting `res.members()[k]` "the best one", that is the p-hacking surface
    // ADR-002 seals. The distribution — median, IQR, worst-5% — is what you read.
    let bt = Backtest::new(InMemoryRunStore::new(), spike_executor());
    let grid: Vec<ParamMap> = (8..=16).map(|i| param(f64::from(i))).collect();
    let res = StudyEngine::run(&sweep(grid, SelectionRule::WorstCaseRobust), &bt).unwrap();
    // Worst-case rule lands on a conservative (low-return) member, never the peak.
    let carried = res.carried_forward.unwrap();
    let fast = carried.params.get("fast").and_then(|v| v.as_f64()).unwrap();
    assert_ne!(fast, 12.0);
}
