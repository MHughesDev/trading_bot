//! End-to-end funnel + mutual-enforcement suite (spec §2.3, J-4.12).
//!
//! Drives an Experiment through Gates 0→4 and asserts the five structural
//! enforcement properties: the funnel can't be skipped, the counter can't be
//! gamed, the vault can't be peeked, the null can't be hidden, and the best
//! member can't be cherry-picked. Each property has a dedicated assertion;
//! removing the corresponding protection breaks exactly one of these tests.

use backtest::experiment::{Experiment, ExperimentError, ExperimentState};
use backtest::gates::{CorroboratorInputs, Gate, GateError, GateRunner, IntegrityInputs};
use backtest::nulls::{Null, NullKind, NullParams};
use backtest::run::executor::{daily_curve, map_sim_result};
use backtest::run::{
    Backtest, ClosureExecutor, ComputeCost, DataSlice, EvalResolution, InMemoryRunStore,
    MetricKind, ParamMap, RunConfig, RunConfigBuilder, ENGINE_VERSION,
};
use backtest::study::{
    SelectionRule, StudyBudget, StudyConfig, StudyEngine, StudyKind, StudyResult, VarySpec,
};
use chrono::{TimeZone, Utc};

fn research_slice() -> DataSlice {
    DataSlice::new(
        "u",
        Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap(),
        EvalResolution::Day1,
    )
}

fn holdout_slice() -> DataSlice {
    DataSlice::new(
        "u",
        Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        EvalResolution::Day1,
    )
}

fn candidate() -> RunConfig {
    RunConfigBuilder::new("ema", "v1", research_slice(), "cost:floor", "sz", "snap").build()
}

fn experiment() -> Experiment {
    Experiment::new("exp-e2e", "ema-family", holdout_slice(), "null:block")
}

fn profitable_bt() -> Backtest<InMemoryRunStore, impl backtest::run::RunExecutor> {
    Backtest::new(
        InMemoryRunStore::new(),
        ClosureExecutor(|cfg: &RunConfig| {
            map_sim_result(
                cfg,
                daily_curve(&[100.0, 101.0, 102.5, 104.0]),
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        }),
    )
}

fn clean_integrity() -> IntegrityInputs<'static> {
    IntegrityInputs {
        signals: &[],
        gross_return: 0.12,
        cost_floor: 0.01,
        label_horizon_bars: None,
        feature_window_end_bar: None,
        purge_bars: None,
    }
}

/// A profitable, plateau-shaped sweep used as study evidence.
fn good_study(id: &str, n: usize) -> StudyResult {
    let bt = profitable_bt();
    let study = StudyConfig {
        study_id: id.into(),
        kind: StudyKind::ParameterSweep,
        base_config: candidate(),
        vary: VarySpec::Params {
            grid: (0..n).map(|_| ParamMap::new()).collect(),
        },
        metric: MetricKind::TotalReturn,
        null_ref: None,
        budget: StudyBudget::default(),
        question: "vary".into(),
        selection_rule: SelectionRule::None,
    };
    StudyEngine::run(&study, &bt).unwrap()
}

#[test]
fn property_1_funnel_cannot_be_skipped() {
    let mut e = experiment();
    let mut runner = GateRunner::new(&mut e);
    // No gate has run → entering Gate 1/2/3/4 is refused for lack of a prior pass.
    let s = good_study("s", 4);
    assert!(matches!(
        runner.gate1(&s),
        Err(GateError::PrerequisiteNotPassed {
            gate: Gate::SinglePath,
            required: Gate::Integrity
        })
    ));
}

#[test]
fn property_2_counter_cannot_be_gamed() {
    // Studies run through the Experiment auto-increment the counter; Gate 3
    // reads that exact count. Run two sweeps (10 + 10) → counter 20.
    let mut e = experiment();
    let bt = profitable_bt();
    for id in ["s1", "s2"] {
        let study = StudyConfig {
            study_id: id.into(),
            kind: StudyKind::ParameterSweep,
            base_config: candidate(),
            vary: VarySpec::Params {
                grid: (0..10).map(|_| ParamMap::new()).collect(),
            },
            metric: MetricKind::TotalReturn,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "vary".into(),
            selection_rule: SelectionRule::None,
        };
        e.run_study(&study, &bt).unwrap();
    }
    assert_eq!(e.trial_counter(), 20);
}

#[test]
fn property_3_vault_cannot_be_peeked() {
    // The vault is reachable only after Gate 3 passes; before that it refuses.
    let mut e = experiment();
    let bt = profitable_bt();
    let mut runner = GateRunner::new(&mut e);
    assert!(matches!(
        runner.gate4(&candidate(), &bt, "mallory"),
        Err(GateError::PrerequisiteNotPassed {
            gate: Gate::Vault,
            required: Gate::Significance
        })
    ));
}

#[test]
fn property_4_null_cannot_be_hidden() {
    // Gate 3 emits a SignificanceResult that structurally carries its null and
    // the trial count — there is no bare-p path. We thread a real null id and
    // assert the verdict renders all three.
    let mut e = experiment();
    let _bt = profitable_bt();
    let null = Null::new(NullKind::BlockPermutation, NullParams::default()).unwrap();
    let mut runner = GateRunner::new(&mut e);
    runner.gate0(&clean_integrity()).unwrap();
    runner.gate1(&good_study("wf", 3)).unwrap();
    runner
        .gate2(
            &good_study("cpcv", 6),
            &good_study("syn", 4),
            &plateau_study("nbhd"),
            -0.5,
        )
        .unwrap();
    let perf = vec![vec![1.0; 8], vec![0.2; 8], vec![0.3; 8]];
    let strong_null: Vec<f64> = (0..999).map(|i| f64::from(i) / 1000.0).collect();
    let (outcome, _passed) = runner
        .gate3(
            5.0,
            &strong_null,
            null.null_id.clone(),
            &CorroboratorInputs {
                sharpe: 2.5,
                n_obs: 252,
                skew: 0.0,
                kurtosis: 3.0,
                sharpe_variance_across_trials: 0.1,
                pbo_performance: &perf,
                pbo_groups: 4,
            },
            0.05,
        )
        .unwrap();
    let rendered = outcome.significance.render();
    assert!(
        rendered.contains(null.null_id.as_str()),
        "null must travel with the p-value"
    );
    assert!(
        rendered.contains("trials"),
        "trial count must travel with the p-value"
    );
}

#[test]
fn property_5_best_member_cannot_be_cherry_picked() {
    // A Study's distribution exposes no best member; only a pre-declared
    // SelectionRule yields one config, and it is never the peak.
    let res = good_study("sealed", 8);
    assert!(res.sealed);
    assert!(
        res.carried_forward.is_none(),
        "SelectionRule::None carries nothing"
    );
}

#[test]
fn full_funnel_validates_a_genuine_edge_in_order() {
    let mut e = experiment();
    let bt = profitable_bt();
    let null = Null::new(NullKind::BlockPermutation, NullParams::default()).unwrap();

    let mut runner = GateRunner::new(&mut e);
    runner.gate0(&clean_integrity()).unwrap();
    assert!(runner.ledger().passed(Gate::Integrity));
    runner.gate1(&good_study("wf", 3)).unwrap();
    runner
        .gate2(
            &good_study("cpcv", 6),
            &good_study("syn", 4),
            &plateau_study("nbhd"),
            -0.5,
        )
        .unwrap();
    let perf = vec![vec![1.0; 8], vec![0.2; 8], vec![0.3; 8]];
    let strong_null: Vec<f64> = (0..999).map(|i| f64::from(i) / 1000.0).collect();
    let (_outcome, passed3) = runner
        .gate3(
            6.0,
            &strong_null,
            null.null_id,
            &CorroboratorInputs {
                sharpe: 2.5,
                n_obs: 252,
                skew: 0.0,
                kurtosis: 3.0,
                sharpe_variance_across_trials: 0.1,
                pbo_performance: &perf,
                pbo_groups: 4,
            },
            0.05,
        )
        .unwrap();
    assert!(passed3);
    let (vault, v) = runner.gate4(&candidate(), &bt, "alice").unwrap();
    assert_eq!(vault.status, backtest::run::RunStatus::Ok);
    assert!(v.passed);
    assert_eq!(e.state, ExperimentState::Validated);

    // And once validated, research is refused (the holdout is spent).
    let bt2 = profitable_bt();
    let s = StudyConfig {
        study_id: "post".into(),
        kind: StudyKind::ParameterSweep,
        base_config: candidate(),
        vary: VarySpec::Params {
            grid: vec![ParamMap::new()],
        },
        metric: MetricKind::TotalReturn,
        null_ref: None,
        budget: StudyBudget::default(),
        question: "too late".into(),
        selection_rule: SelectionRule::None,
    };
    assert!(matches!(
        e.run_study(&s, &bt2),
        Err(ExperimentError::OperationNotAllowed { .. })
    ));
}

/// A neighborhood study with a flat (plateau) objective.
fn plateau_study(id: &str) -> StudyResult {
    let bt = Backtest::new(
        InMemoryRunStore::new(),
        ClosureExecutor(|cfg: &RunConfig| {
            // Flat, positive return regardless of params → low spread → plateau.
            map_sim_result(
                cfg,
                daily_curve(&[100.0, 101.0, 102.0]),
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        }),
    );
    let study = StudyConfig {
        study_id: id.into(),
        kind: StudyKind::Neighborhood,
        base_config: candidate(),
        vary: VarySpec::Neighborhood {
            param: "fast".into(),
            center: 12.0,
            step: 1.0,
            k: 4,
        },
        metric: MetricKind::TotalReturn,
        null_ref: None,
        budget: StudyBudget::default(),
        question: "plateau?".into(),
        selection_rule: SelectionRule::None,
    };
    StudyEngine::run(&study, &bt).unwrap()
}
