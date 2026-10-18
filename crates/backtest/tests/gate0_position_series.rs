//! Gate 0 refuses a Layer 1 experiment that has no truncation verdict (Set M M-1.7, ST-03).
//!
//! The property is structural: the plain `gate0` call on a Layer 1 experiment carries no
//! verdict, so it fails with a flag rather than skipping the check, and the only way past it
//! is a verdict that passed. A leaky function fails and names where.

use backtest::experiment::Experiment;
use backtest::gates::{position_leakage, GateError, GateRunner, IntegrityInputs};
use backtest::position_series::{
    truncation_test, Bar, PositionPoint, PositionSeries, PositionUnits, TruncationVerdict,
};
use backtest::run::{DataSlice, EvalResolution};
use chrono::{TimeZone, Utc};

fn holdout() -> DataSlice {
    DataSlice::new(
        "u",
        Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        EvalResolution::Day1,
    )
}

fn clean() -> IntegrityInputs<'static> {
    IntegrityInputs {
        signals: &[],
        gross_return: 0.12,
        cost_floor: 0.01,
        label_horizon_bars: None,
        feature_window_end_bar: None,
        purge_bars: None,
    }
}

fn layer1_experiment() -> Experiment {
    Experiment::new("exp-l1", format!("{}momo", position_leakage::LAYER1_FAMILY_PREFIX), holdout(), "null:block")
}

fn bars(n: usize) -> Vec<Bar> {
    (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let p = 100.0 + (i as f64 * 0.37).sin() * 3.0 + i as f64 * 0.01;
            Bar { ts_ns: i64::try_from(i).unwrap() * 3_600_000_000_000, open: p, close: p + 0.1 }
        })
        .collect()
}

fn series_from(bars: &[Bar], f: impl Fn(usize) -> f64) -> PositionSeries {
    PositionSeries {
        points: bars.iter().enumerate().map(|(i, b)| PositionPoint::new(b.ts_ns, f(i))).collect(),
        position_units: PositionUnits::Fraction,
        allow_short: false,
    }
}

fn causal(b: &[Bar]) -> PositionSeries {
    // Long when the close is above its previous close: reads only the past.
    series_from(b, |i| if i > 0 && b[i].close > b[i - 1].close { 1.0 } else { 0.0 })
}

fn peeking(b: &[Bar]) -> PositionSeries {
    // Long when the NEXT close is higher: reads the bar after the decision.
    series_from(b, |i| if i + 1 < b.len() && b[i + 1].close > b[i].close { 1.0 } else { 0.0 })
}

#[test]
fn a_layer1_experiment_cannot_pass_gate_0_without_a_truncation_verdict() {
    let mut e = layer1_experiment();
    let mut runner = GateRunner::new(&mut e);
    // The plain call carries no verdict. It must not be a way around the check.
    assert!(matches!(runner.gate0(&clean()), Err(GateError::IntegrityHardStop)));
    let v = runner.ledger().verdicts().last().expect("the failure is recorded");
    assert!(!v.passed);
    assert!(v.reasons.iter().any(|r| r == position_leakage::NOT_RUN_CODE), "{:?}", v.reasons);
    // And a call that names no verdict is the same refusal, not a skip.
    let mut e = layer1_experiment();
    let mut runner = GateRunner::new(&mut e);
    assert!(runner.gate0_position_series(&clean(), None).is_err());
}

#[test]
fn a_passing_verdict_lets_a_layer1_experiment_through_and_a_failing_one_names_where() {
    let b = bars(400);
    let ok = truncation_test(causal, &b, 7, 8);
    assert!(ok.passed(), "{ok:?}");
    let mut e = layer1_experiment();
    let mut runner = GateRunner::new(&mut e);
    assert!(runner.gate0_position_series(&clean(), Some(&ok)).is_ok());

    let leaky = truncation_test(peeking, &b, 7, 8);
    let TruncationVerdict::Fail(f) = &leaky else { panic!("a peeking function must fail: {leaky:?}") };
    let mut e = layer1_experiment();
    let mut runner = GateRunner::new(&mut e);
    assert!(runner.gate0_position_series(&clean(), Some(&leaky)).is_err());
    let verdict = runner.ledger().verdicts().last().unwrap();
    assert!(verdict.reasons.iter().any(|r| r == position_leakage::CODE));
    let flag = position_leakage::check(Some(&leaky)).unwrap();
    assert!(
        flag.detail.contains(&chrono::DateTime::from_timestamp_nanos(f.first_differing_ns).to_rfc3339()),
        "the flag names the first differing timestamp: {}",
        flag.detail
    );
}

#[test]
fn an_ordinary_experiment_is_not_asked_for_a_verdict() {
    let mut e = Experiment::new("exp-plain", "ema-family", holdout(), "null:block");
    let mut runner = GateRunner::new(&mut e);
    assert!(runner.gate0(&clean()).is_ok(), "the check applies to Layer 1 only");
}
