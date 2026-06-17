//! The reconciliation loop (spec §2.2 tail, Phase 5).
//!
//! Once `live`, the only Studies permitted compare realized performance against
//! the backtested distribution for the same period. When realized performance
//! drifts below the backtest's predicted distribution (under the worst-5% you
//! planned around), the Experiment auto-transitions to `decaying`. Aggregated
//! across validated Experiments, reconciliation tells you whether the *whole
//! suite* is calibrated — the meta-signal that catches the overfit that survived
//! every gate.

use serde::{Deserialize, Serialize};

use crate::experiment::{Experiment, ExperimentError, ExperimentState, Operation};
use crate::study::Distribution;

/// Where a realized value sits within a backtested distribution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationPoint {
    /// The realized performance value.
    pub realized: f64,
    /// Its percentile within the backtest distribution, in `[0, 1]`.
    pub percentile: f64,
    /// True if it fell below the planned worst-5% of the backtest.
    pub below_worst_5pct: bool,
}

/// Locate a realized value within a backtested distribution.
#[must_use]
pub fn reconcile_point(realized: f64, backtest: &Distribution) -> ReconciliationPoint {
    let dist = &backtest.dist;
    let pct = if dist.is_empty() {
        0.5
    } else {
        dist.iter().filter(|&&x| x <= realized).count() as f64 / dist.len() as f64
    };
    ReconciliationPoint {
        realized,
        percentile: pct,
        below_worst_5pct: realized < backtest.worst_5pct,
    }
}

/// The verdict of a reconciliation Study over several realized periods.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReconciliationVerdict {
    pub points: Vec<ReconciliationPoint>,
    /// Fraction of periods that fell below the planned worst-5%.
    pub fraction_below_worst5: f64,
    /// True once drift is severe enough to flag decay.
    pub drifting: bool,
}

/// A reconciliation Study: compare realized returns against the backtest
/// distribution for the same span. Allowed only in `live`/`decaying` (the caller
/// enforces state via [`reconcile_experiment`]). `drift_threshold` is the share
/// of periods below worst-5% that triggers a decay flag (e.g. 0.10).
#[must_use]
pub fn reconciliation_verdict(
    realized: &[f64],
    backtest: &Distribution,
    drift_threshold: f64,
) -> ReconciliationVerdict {
    let points: Vec<ReconciliationPoint> = realized
        .iter()
        .map(|&r| reconcile_point(r, backtest))
        .collect();
    let below = points.iter().filter(|p| p.below_worst_5pct).count();
    let fraction = if points.is_empty() {
        0.0
    } else {
        below as f64 / points.len() as f64
    };
    ReconciliationVerdict {
        points,
        fraction_below_worst5: fraction,
        drifting: fraction > drift_threshold,
    }
}

/// Run reconciliation against a `live`/`decaying` Experiment and auto-transition
/// it to `decaying` when drift is detected (spec §2.2 tail / J-5.2).
///
/// # Errors
/// [`ExperimentError::OperationNotAllowed`] if the Experiment is not live/decaying.
pub fn reconcile_experiment(
    experiment: &mut Experiment,
    realized: &[f64],
    backtest: &Distribution,
    drift_threshold: f64,
) -> Result<ReconciliationVerdict, ExperimentError> {
    if !experiment.state.allows(Operation::ReconciliationStudy) {
        return Err(ExperimentError::OperationNotAllowed {
            state: experiment.state,
            op: Operation::ReconciliationStudy,
        });
    }
    let verdict = reconciliation_verdict(realized, backtest, drift_threshold);
    if verdict.drifting && experiment.state == ExperimentState::Live {
        // can_transition_to(Live -> Decaying) is legal.
        let _ = experiment.transition(ExperimentState::Decaying);
    }
    Ok(verdict)
}

/// Suite-level calibration: are validated strategies, on average, landing where
/// their backtests predicted? Under perfect calibration the realized percentiles
/// are uniform on `[0,1]`; a left-skewed mean (< 0.5) means the suite is
/// systematically optimistic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuiteCalibration {
    pub n_points: usize,
    /// Mean realized percentile across all experiments (0.5 ≈ calibrated).
    pub mean_percentile: f64,
    /// Coverage of the planned worst-5% line: the share of points at or above it
    /// (≈ 0.95 when calibrated).
    pub worst5_coverage: f64,
    /// True when the suite is systematically optimistic (realized below predicted).
    pub optimistic: bool,
}

/// Aggregate reconciliation points across all validated Experiments (J-5.3).
#[must_use]
pub fn suite_calibration(points: &[ReconciliationPoint]) -> SuiteCalibration {
    let n = points.len();
    if n == 0 {
        return SuiteCalibration {
            n_points: 0,
            mean_percentile: 0.5,
            worst5_coverage: 1.0,
            optimistic: false,
        };
    }
    let mean = points.iter().map(|p| p.percentile).sum::<f64>() / n as f64;
    let covered = points.iter().filter(|p| !p.below_worst_5pct).count() as f64 / n as f64;
    SuiteCalibration {
        n_points: n,
        mean_percentile: mean,
        worst5_coverage: covered,
        // Optimistic if realized lands well below predicted on average, or the
        // worst-5% line is breached far more than the planned 5%.
        optimistic: mean < 0.40 || covered < 0.90,
    }
}

/// The empirical CDF value (PIT) of `x` in `samples` — exposed for the
/// frontend's reliability/PIT chart (J-5.3 UI consumes this).
#[must_use]
pub fn pit(x: f64, samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.5;
    }
    // PIT is the empirical CDF: the fraction of samples at or below x.
    samples.iter().filter(|&&s| s <= x).count() as f64 / samples.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::ExperimentState;
    use crate::run::{DataSlice, EvalResolution, MetricKind};
    use chrono::{TimeZone, Utc};

    fn backtest_dist() -> Distribution {
        // Backtested OOS returns centered around +0.02, worst-5% ≈ -0.01.
        Distribution::from_values(
            MetricKind::TotalReturn,
            vec![-0.01, 0.0, 0.01, 0.02, 0.02, 0.03, 0.03, 0.04, 0.05, 0.06],
        )
    }

    fn live_experiment() -> Experiment {
        let holdout = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        let mut e = Experiment::new("exp", "fam", holdout, "null:x");
        // Walk it to live: candidate→validated→live.
        e.transition(ExperimentState::Validated).unwrap();
        e.transition(ExperimentState::Live).unwrap();
        e
    }

    #[test]
    fn reconcile_point_locates_within_distribution() {
        let d = backtest_dist();
        let high = reconcile_point(0.06, &d);
        assert!(high.percentile > 0.9);
        assert!(!high.below_worst_5pct);
        let low = reconcile_point(-0.05, &d);
        assert!(low.below_worst_5pct);
    }

    #[test]
    fn reconciliation_only_in_live_or_decaying() {
        let holdout = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        let mut candidate = Experiment::new("exp", "fam", holdout, "null:x");
        let d = backtest_dist();
        assert!(matches!(
            reconcile_experiment(&mut candidate, &[0.01], &d, 0.1),
            Err(ExperimentError::OperationNotAllowed { .. })
        ));
    }

    #[test]
    fn drift_below_worst5_transitions_to_decaying() {
        let mut e = live_experiment();
        let d = backtest_dist();
        // A run of realized returns well below the planned worst-5%.
        let realized = vec![-0.05, -0.06, -0.04, -0.05, -0.07];
        let verdict = reconcile_experiment(&mut e, &realized, &d, 0.10).unwrap();
        assert!(verdict.drifting);
        assert_eq!(e.state, ExperimentState::Decaying);
    }

    #[test]
    fn in_distribution_performance_stays_live() {
        let mut e = live_experiment();
        let d = backtest_dist();
        let realized = vec![0.02, 0.03, 0.01, 0.02, 0.04];
        let verdict = reconcile_experiment(&mut e, &realized, &d, 0.10).unwrap();
        assert!(!verdict.drifting);
        assert_eq!(e.state, ExperimentState::Live);
    }

    #[test]
    fn suite_calibration_flags_systematic_optimism() {
        // All realized points sit in the low tail → optimistic suite.
        let pts: Vec<ReconciliationPoint> = (0..20)
            .map(|_| ReconciliationPoint {
                realized: -0.05,
                percentile: 0.05,
                below_worst_5pct: true,
            })
            .collect();
        let cal = suite_calibration(&pts);
        assert!(cal.optimistic);
        assert!(cal.worst5_coverage < 0.9);

        // Uniformly-spread points → calibrated.
        let good: Vec<ReconciliationPoint> = (0..20)
            .map(|i| ReconciliationPoint {
                realized: 0.0,
                percentile: f64::from(i) / 20.0,
                below_worst_5pct: i == 0, // ~5% below
            })
            .collect();
        let cal2 = suite_calibration(&good);
        assert!(!cal2.optimistic, "mean={} cov={}", cal2.mean_percentile, cal2.worst5_coverage);
    }
}
