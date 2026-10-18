//! Research-to-deploy reconciliation: does the Layer 2 version do what the Layer 1
//! idea did? (Set M, FEAT-004 §7, ST-10.)
//!
//! A Layer 1 idea is free-form Python and a Layer 2 version is the typed,
//! compiled strategy that could actually be deployed. Translating one into the
//! other is where an idea quietly becomes a different idea: a window off by one,
//! a rounding rule, a warm-up that is handled differently. The reconciliation
//! runs both on the same window under the same policy and costs and asks two
//! numbers and one question:
//!
//! * **agreement**: the fraction of decision times at which the two target
//!   positions are identical;
//! * **tracking error**: the standard deviation of the per-bar difference between
//!   the two equity curves' returns;
//! * and *is every disagreement inside a declared approximation class?*
//!
//! A pass needs all three. The last is not redundant with the first: 99.9%
//! agreement with one disagreement nobody can explain is the dangerous case, so an
//! unexplained disagreement fails the reconciliation however high the agreement.
//!
//! # No defaults for the thresholds
//!
//! FEAT-004 gives `recon.agreement_min` a default of 0.99 and says nothing about
//! the tracking-error bound (open question 2: it depends on the timeframe).
//! [`ReconcileThresholds::new`] therefore takes both, validated, and there is no
//! `Default`: a bound nobody chose is a bound that passes whatever it is shown.
//!
//! The verdict is recorded in `strategy_reconciliation` (migration 0205), whose
//! CHECK refuses a `passed = true` row that its own recorded thresholds contradict.

use serde::Serialize;

use crate::position_series::PositionSeries;

/// A declared reason two positions may legitimately differ.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ApproximationClass {
    /// Before the later of the two strategies has enough history to decide.
    Warmup { until_ns: i64 },
    /// Lot or tick rounding: positions within `abs_tol` of each other.
    Rounding { abs_tol: f64 },
    /// A named window the author declares is expected to differ (a known v1 bug,
    /// a venue halt), with the reason carried into the record.
    Window {
        start_ns: i64,
        end_ns: i64,
        reason: String,
    },
}

impl ApproximationClass {
    fn name(&self) -> String {
        match self {
            Self::Warmup { .. } => "warmup".into(),
            Self::Rounding { .. } => "rounding".into(),
            Self::Window { reason, .. } => format!("window: {reason}"),
        }
    }

    fn explains(&self, ts: i64, a: f64, b: f64) -> bool {
        match self {
            Self::Warmup { until_ns } => ts < *until_ns,
            Self::Rounding { abs_tol } => (a - b).abs() <= *abs_tol,
            Self::Window {
                start_ns, end_ns, ..
            } => ts >= *start_ns && ts < *end_ns,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ReconcileThresholds {
    pub agreement_min: f64,
    pub te_max: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ReconcileError {
    #[error("agreement_min must be in (0, 1], got {0}")]
    BadAgreementMin(f64),
    #[error("te_max must be finite and non-negative, got {0}")]
    BadTeMax(f64),
    #[error("the two series are on different decision clocks ({a} and {b} points, or different timestamps)")]
    DifferentClocks { a: usize, b: usize },
    #[error("the two equity curves are on different clocks or too short to compare")]
    DifferentEquity,
    #[error("a target or equity value is not finite")]
    NonFinite,
}

impl ReconcileThresholds {
    /// # Errors
    /// An agreement bound outside `(0, 1]` or a negative or non-finite tracking-error bound.
    pub fn new(agreement_min: f64, te_max: f64) -> Result<Self, ReconcileError> {
        if !(agreement_min > 0.0 && agreement_min <= 1.0) {
            return Err(ReconcileError::BadAgreementMin(agreement_min));
        }
        if !(te_max.is_finite() && te_max >= 0.0) {
            return Err(ReconcileError::BadTeMax(te_max));
        }
        Ok(Self {
            agreement_min,
            te_max,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Disagreement {
    pub ts_ns: i64,
    pub layer1: f64,
    pub layer2: f64,
    /// The first declared class that explains it; `None` is unexplained.
    pub class: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reconciliation {
    pub n_decisions: usize,
    pub agreement: f64,
    pub pnl_tracking_error: f64,
    pub disagreements: Vec<Disagreement>,
    pub unexplained: usize,
    pub thresholds: ReconcileThresholds,
    pub passed: bool,
}

fn returns(equity: &[(i64, f64)]) -> Result<Vec<f64>, ReconcileError> {
    if equity.len() < 3 {
        return Err(ReconcileError::DifferentEquity);
    }
    let mut out = Vec::with_capacity(equity.len() - 1);
    for w in equity.windows(2) {
        if !(w[0].1.is_finite() && w[1].1.is_finite()) {
            return Err(ReconcileError::NonFinite);
        }
        if w[0].1 <= 0.0 {
            return Err(ReconcileError::DifferentEquity);
        }
        out.push(w[1].1 / w[0].1 - 1.0);
    }
    Ok(out)
}

/// Reconcile a Layer 1 series against a Layer 2 series, with the two equity
/// curves each was evaluated to under one policy.
///
/// # Errors
/// Series or equity curves on different clocks, or non-finite values.
pub fn reconcile(
    layer1: &PositionSeries,
    layer2: &PositionSeries,
    equity1: &[(i64, f64)],
    equity2: &[(i64, f64)],
    classes: &[ApproximationClass],
    thresholds: ReconcileThresholds,
) -> Result<Reconciliation, ReconcileError> {
    let (a, b) = (layer1.effective(), layer2.effective());
    if a.len() != b.len() || a.iter().zip(&b).any(|(x, y)| x.0 != y.0) {
        return Err(ReconcileError::DifferentClocks {
            a: a.len(),
            b: b.len(),
        });
    }
    if a.is_empty() {
        return Err(ReconcileError::DifferentClocks { a: 0, b: 0 });
    }
    if equity1.len() != equity2.len() || equity1.iter().zip(equity2).any(|(x, y)| x.0 != y.0) {
        return Err(ReconcileError::DifferentEquity);
    }
    let (r1, r2) = (returns(equity1)?, returns(equity2)?);
    let diffs: Vec<f64> = r1.iter().zip(&r2).map(|(x, y)| x - y).collect();
    let m = diffs.iter().sum::<f64>() / diffs.len() as f64;
    let te =
        (diffs.iter().map(|d| (d - m).powi(2)).sum::<f64>() / (diffs.len() as f64 - 1.0)).sqrt();

    let mut disagreements = Vec::new();
    for ((ts, x), (_, y)) in a.iter().zip(&b) {
        if !(x.is_finite() && y.is_finite()) {
            return Err(ReconcileError::NonFinite);
        }
        // Layer agreement is bit-exact by definition: any difference is a disagreement.
        #[allow(clippy::float_cmp)]
        let differs = x != y;
        if differs {
            let class = classes
                .iter()
                .find(|c| c.explains(*ts, *x, *y))
                .map(ApproximationClass::name);
            disagreements.push(Disagreement {
                ts_ns: *ts,
                layer1: *x,
                layer2: *y,
                class,
            });
        }
    }
    let n = a.len();
    let agreement = (n - disagreements.len()) as f64 / n as f64;
    let unexplained = disagreements.iter().filter(|d| d.class.is_none()).count();
    let passed =
        agreement >= thresholds.agreement_min && te <= thresholds.te_max && unexplained == 0;
    Ok(Reconciliation {
        n_decisions: n,
        agreement,
        pnl_tracking_error: te,
        disagreements,
        unexplained,
        thresholds,
        passed,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::position_series::{evaluate, Bar, Costs, EvalConfig, PositionPoint, PositionUnits};

    fn bars(n: usize) -> Vec<Bar> {
        let mut p = 100.0_f64;
        (0..n)
            .map(|i| {
                let open = p * (1.0 + 0.001 * ((i % 2) as f64 + 1.0));
                p = open * (1.0 + 0.01 * (((i * 7 + 3) % 11) as f64 - 5.0) / 5.0);
                Bar {
                    ts_ns: (i as i64 + 1) * 60_000_000_000,
                    open,
                    close: p,
                }
            })
            .collect()
    }

    fn series(b: &[Bar], f: impl Fn(usize) -> f64) -> PositionSeries {
        PositionSeries {
            position_units: PositionUnits::Fraction,
            allow_short: false,
            points: b
                .iter()
                .enumerate()
                .map(|(i, x)| PositionPoint::new(x.ts_ns, f(i)))
                .collect(),
        }
    }

    fn cfg() -> EvalConfig {
        EvalConfig {
            initial_equity: 10_000.0,
            costs: Costs {
                fee_bps: 10.0,
                half_spread_bps: 2.0,
            },
            lot_size: None,
        }
    }

    fn run(b: &[Bar], s: &PositionSeries) -> Vec<(i64, f64)> {
        evaluate(s, b, &cfg()).unwrap().equity
    }

    fn th() -> ReconcileThresholds {
        ReconcileThresholds::new(0.99, 1e-4).unwrap()
    }

    /// A rule both layers implement: long every third bar.
    fn rule(i: usize) -> f64 {
        if i % 6 < 3 {
            1.0
        } else {
            0.0
        }
    }

    #[test]
    fn a_faithful_translation_passes() {
        let b = bars(600);
        let (l1, l2) = (series(&b, rule), series(&b, rule));
        let r = reconcile(&l1, &l2, &run(&b, &l1), &run(&b, &l2), &[], th()).unwrap();
        assert_eq!(r.agreement, 1.0);
        assert_eq!(r.pnl_tracking_error, 0.0);
        assert!(r.passed);
        assert!(r.disagreements.is_empty());
    }

    #[test]
    fn a_translation_off_by_one_bar_is_blocked_and_lists_where() {
        let b = bars(600);
        let l1 = series(&b, rule);
        // The "translation" decides one bar late.
        let l2 = series(&b, |i| if i == 0 { 0.0 } else { rule(i - 1) });
        let r = reconcile(&l1, &l2, &run(&b, &l1), &run(&b, &l2), &[], th()).unwrap();
        assert!(!r.passed);
        assert!(r.agreement < 0.99, "agreement {}", r.agreement);
        assert!(!r.disagreements.is_empty());
        assert!(r.disagreements.iter().all(|d| d.class.is_none()));
        assert!(r.pnl_tracking_error > 1e-4);
    }

    #[test]
    fn an_unexplained_disagreement_fails_however_high_the_agreement() {
        let b = bars(1000);
        let l1 = series(&b, rule);
        let l2 = series(&b, |i| if i == 500 { 1.0 - rule(i) } else { rule(i) });
        // One disagreement in 1000 is 99.9% agreement, well above 0.99.
        let loose = ReconcileThresholds::new(0.99, 1.0).unwrap();
        let r = reconcile(&l1, &l2, &run(&b, &l1), &run(&b, &l2), &[], loose).unwrap();
        assert!(r.agreement >= 0.999);
        assert_eq!(r.unexplained, 1);
        assert!(!r.passed, "an unexplained difference is the dangerous case");
    }

    #[test]
    fn a_declared_class_explains_a_disagreement_and_the_record_carries_which() {
        let b = bars(1000);
        let l1 = series(&b, rule);
        let l2 = series(&b, |i| if i < 20 { 0.0 } else { rule(i) });
        let warm = [ApproximationClass::Warmup {
            until_ns: b[20].ts_ns,
        }];
        let loose = ReconcileThresholds::new(0.9, 1.0).unwrap();
        let r = reconcile(&l1, &l2, &run(&b, &l1), &run(&b, &l2), &warm, loose).unwrap();
        assert!(!r.disagreements.is_empty());
        assert_eq!(r.unexplained, 0);
        assert!(r
            .disagreements
            .iter()
            .all(|d| d.class.as_deref() == Some("warmup")));
        assert!(r.passed);

        // The same disagreement with no class declared fails.
        let r = reconcile(&l1, &l2, &run(&b, &l1), &run(&b, &l2), &[], loose).unwrap();
        assert!(!r.passed);

        // A rounding class explains a small difference and not a large one.
        let rounding = [ApproximationClass::Rounding { abs_tol: 0.01 }];
        let tiny = series(&b, |i| rule(i) * 0.995);
        let r = reconcile(&l1, &tiny, &run(&b, &l1), &run(&b, &tiny), &rounding, loose).unwrap();
        assert!(
            r.disagreements
                .iter()
                .any(|d| d.class.as_deref() == Some("rounding"))
                || r.agreement == 1.0
        );
        assert_eq!(r.unexplained, 0);
    }

    #[test]
    fn a_declared_window_carries_its_reason() {
        let b = bars(300);
        let l1 = series(&b, rule);
        let l2 = series(&b, |i| {
            if (100..110).contains(&i) {
                0.0
            } else {
                rule(i)
            }
        });
        let w = [ApproximationClass::Window {
            start_ns: b[100].ts_ns,
            end_ns: b[110].ts_ns,
            reason: "v1 held through the halt".into(),
        }];
        let loose = ReconcileThresholds::new(0.9, 1.0).unwrap();
        let r = reconcile(&l1, &l2, &run(&b, &l1), &run(&b, &l2), &w, loose).unwrap();
        assert_eq!(r.unexplained, 0);
        assert!(r
            .disagreements
            .iter()
            .all(|d| d.class.as_deref().is_some_and(|c| c.contains("halt"))));
    }

    #[test]
    fn tracking_error_alone_can_fail_a_series_that_agrees_everywhere() {
        let b = bars(300);
        let l = series(&b, rule);
        let mut skew = run(&b, &l);
        for (i, e) in skew.iter_mut().enumerate() {
            e.1 *= 1.0 + 0.002 * ((i % 2) as f64); // the second evaluation drifts noisily
        }
        let r = reconcile(&l, &l, &run(&b, &l), &skew, &[], th()).unwrap();
        assert_eq!(r.agreement, 1.0);
        assert!(r.pnl_tracking_error > 1e-4);
        assert!(!r.passed);
    }

    #[test]
    fn thresholds_have_no_default_and_are_validated() {
        assert_eq!(
            ReconcileThresholds::new(0.0, 0.1),
            Err(ReconcileError::BadAgreementMin(0.0))
        );
        assert_eq!(
            ReconcileThresholds::new(1.5, 0.1),
            Err(ReconcileError::BadAgreementMin(1.5))
        );
        assert!(matches!(
            ReconcileThresholds::new(0.99, -1.0),
            Err(ReconcileError::BadTeMax(_))
        ));
        assert!(matches!(
            ReconcileThresholds::new(0.99, f64::NAN),
            Err(ReconcileError::BadTeMax(_))
        ));
        assert!(ReconcileThresholds::new(1.0, 0.0).is_ok());
    }

    #[test]
    fn series_on_different_clocks_are_refused_not_aligned() {
        let b = bars(100);
        let l1 = series(&b, rule);
        let l2 = series(&b[..90], rule);
        let e1 = run(&b, &l1);
        let e2 = run(&b[..90], &l2);
        assert!(matches!(
            reconcile(&l1, &l2, &e1, &e2, &[], th()),
            Err(ReconcileError::DifferentClocks { .. })
        ));
        let l3 = series(&b, rule);
        assert_eq!(
            reconcile(&l1, &l3, &e1, &e1[..50], &[], th()),
            Err(ReconcileError::DifferentEquity)
        );
    }
}
