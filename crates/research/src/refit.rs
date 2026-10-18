//! Campaign Phase 4 pieces: budget across instruments, refit cadence, calibration-drift
//! detection, the cost-model ladder, and the sweep parallelism limit (Set R-b RB-5.8,
//! RB-5.9; FEAT-003 §7.5, §7.6, §15).
//!
//! Pure functions. The driver and the inbox are in `crates/api`; this crate holds the
//! rules so they are one definition the driver, the tests and the report share.
//!
//! # Contents
//!
//! * [`split_budget`]: a campaign's trial budget over several instruments, equally,
//!   with the remainder given to the earliest instruments and *never more than the
//!   budget in total*.
//! * [`refit_due`]: a cadence check shared by the regime refit and model refits.
//! * [`coverage_drift`]: a forecaster whose 50% and 90% intervals stop covering what
//!   they claim to cover is drifting; two consecutive windows outside tolerance raise
//!   an alert. One window is noise.
//! * [`cost_ladder_verdict`]: a sweep run at rising cost multipliers; the strategy
//!   passes the cost gate input only if it still earns at the required multiplier, and
//!   the verdict reports the breakeven multiplier.
//! * [`sweep_slots`] and [`sweep_max_parallel`]: sweeps are capped (default 4) and
//!   always leave one slot for interactive backtests, so a campaign cannot starve them.
//! * [`racing_report`]: racing between fidelity tiers is enabled only where the measured
//!   rank correlation is at least 0.8, and the report says where it is off and why.

use crate::calibration::{FidelityCalibration, RACING_MIN_RHO};

/// The default for `sweep_max_parallel` (FEAT-003 §7.6).
pub const DEFAULT_SWEEP_MAX_PARALLEL: usize = 4;

/// `sweep_max_parallel` from the environment, else the default. A value below 1 is the
/// default: zero parallelism would be a campaign that never starts.
#[must_use]
pub fn sweep_max_parallel() -> usize {
    std::env::var("SWEEP_MAX_PARALLEL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(DEFAULT_SWEEP_MAX_PARALLEL)
}

/// How many backtest slots sweeps may hold at once, out of `capacity`: at most
/// `max_parallel`, and always leaving one slot free so an interactive backtest is never
/// queued behind a whole campaign. With a single slot the sweeps take it and interactive
/// work is the exception (nothing else can be offered), so the floor is one.
#[must_use]
pub fn sweep_slots(capacity: usize, max_parallel: usize) -> usize {
    max_parallel.min(capacity.saturating_sub(1)).max(1)
}

/// Split `budget` trials across `instruments` instruments. The remainder goes one each
/// to the first instruments; the parts always sum to the budget.
#[must_use]
pub fn split_budget(budget: u32, instruments: usize) -> Vec<u32> {
    if instruments == 0 {
        return Vec::new();
    }
    let n = u32::try_from(instruments).unwrap_or(u32::MAX);
    let (base, rem) = (budget / n, budget % n);
    (0..n).map(|i| base + u32::from(i < rem)).collect()
}

/// Whether a refit is due: `days_since_fit` at or past `period_days`, or never fitted.
#[must_use]
pub fn refit_due(days_since_fit: Option<i64>, period_days: i64) -> bool {
    days_since_fit.is_none_or(|d| d >= period_days)
}

/// One window of a forecaster's interval coverage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverageWindow {
    pub coverage_50: f64,
    pub coverage_90: f64,
    pub n: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drift {
    /// Index of the second consecutive bad window.
    pub at_window: usize,
    pub message: String,
}

/// Tolerances for [`coverage_drift`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriftTolerance {
    pub tol_50: f64,
    pub tol_90: f64,
    /// A window with fewer forecasts than this carries too little to judge and is
    /// treated as neither good nor bad (it breaks no run and extends none).
    pub min_n: u32,
}

impl Default for DriftTolerance {
    fn default() -> Self {
        Self {
            tol_50: 0.10,
            tol_90: 0.05,
            min_n: 30,
        }
    }
}

/// The first point at which two consecutive judged windows both lie outside tolerance.
#[must_use]
pub fn coverage_drift(series: &[CoverageWindow], tol: &DriftTolerance) -> Option<Drift> {
    let mut prev_bad = false;
    for (i, w) in series.iter().enumerate() {
        if w.n < tol.min_n {
            continue;
        }
        let bad = (w.coverage_50 - 0.5).abs() > tol.tol_50 || (w.coverage_90 - 0.9).abs() > tol.tol_90;
        if bad && prev_bad {
            return Some(Drift {
                at_window: i,
                message: format!(
                    "interval coverage left tolerance in two consecutive windows: 50% interval covered {:.2} (tolerance {:.2}), 90% interval covered {:.2} (tolerance {:.2})",
                    w.coverage_50, tol.tol_50, w.coverage_90, tol.tol_90
                ),
            });
        }
        prev_bad = bad;
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct CostVerdict {
    pub passes: bool,
    /// Highest tested multiplier at which the score is still positive.
    pub last_positive: Option<f64>,
    /// Interpolated multiplier where the score crosses zero, when it does inside the ladder.
    pub breakeven: Option<f64>,
}

/// `ladder` is `(cost multiplier, score)` in ascending multiplier order. The strategy
/// passes if it still earns (score above zero) at `required` and at every lower tested
/// multiplier.
#[must_use]
pub fn cost_ladder_verdict(ladder: &[(f64, f64)], required: f64) -> CostVerdict {
    let last_positive = ladder.iter().take_while(|(_, s)| *s > 0.0).last().map(|(m, _)| *m);
    let mut breakeven = None;
    for w in ladder.windows(2) {
        let ((m0, s0), (m1, s1)) = (w[0], w[1]);
        if s0 > 0.0 && s1 <= 0.0 {
            breakeven = Some(m0 + (m1 - m0) * s0 / (s0 - s1));
            break;
        }
    }
    let passes = ladder.iter().any(|(m, _)| *m >= required)
        && ladder.iter().filter(|(m, _)| *m <= required).all(|(_, s)| *s > 0.0)
        && !ladder.is_empty();
    CostVerdict { passes, last_positive, breakeven }
}

/// The racing report: which adjacent tiers may race and which may not, and why.
#[must_use]
pub fn racing_report(calibrations: &[FidelityCalibration]) -> Vec<String> {
    calibrations
        .iter()
        .map(|c| {
            if c.racing_enabled {
                format!("racing {} -> {}: on (rho {:.2} over {} configurations)", c.tier_from, c.tier_to, c.rho, c.n)
            } else if c.rho.is_nan() {
                format!("racing {} -> {}: off (too few configurations to measure rho; need {RACING_MIN_RHO} once measured)", c.tier_from, c.tier_to)
            } else {
                format!("racing {} -> {}: off (rho {:.2} is below {RACING_MIN_RHO}); full fidelity is used", c.tier_from, c.tier_to, c.rho)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_splits_exactly_and_never_exceeds_itself() {
        assert_eq!(split_budget(10, 3), vec![4, 3, 3]);
        assert_eq!(split_budget(2, 5), vec![1, 1, 0, 0, 0]);
        for (b, n) in [(200u32, 7usize), (1, 1), (0, 4), (999, 13)] {
            assert_eq!(split_budget(b, n).iter().sum::<u32>(), b);
        }
        assert!(split_budget(10, 0).is_empty());
    }

    #[test]
    fn sweeps_are_capped_and_leave_one_slot_for_interactive_work() {
        assert_eq!(DEFAULT_SWEEP_MAX_PARALLEL, 4);
        assert_eq!(sweep_slots(16, 4), 4, "the cap binds");
        assert_eq!(sweep_slots(4, 4), 3, "one slot stays free");
        assert_eq!(sweep_slots(2, 4), 1);
        assert_eq!(sweep_slots(1, 4), 1, "the floor is one: a campaign must be able to start");
        assert_eq!(sweep_slots(0, 4), 1);
        std::env::remove_var("SWEEP_MAX_PARALLEL");
        assert_eq!(sweep_max_parallel(), 4);
    }

    #[test]
    fn refit_is_due_when_never_fitted_or_the_period_has_passed() {
        assert!(refit_due(None, 30));
        assert!(!refit_due(Some(29), 30));
        assert!(refit_due(Some(30), 30));
    }

    fn w(c50: f64, c90: f64, n: u32) -> CoverageWindow {
        CoverageWindow { coverage_50: c50, coverage_90: c90, n }
    }

    #[test]
    fn one_bad_window_is_noise_and_two_in_a_row_raise_the_alert() {
        let t = DriftTolerance::default();
        let good = w(0.5, 0.9, 100);
        let bad = w(0.7, 0.8, 100);
        assert_eq!(coverage_drift(&[good, bad, good, bad, good], &t), None);
        let d = coverage_drift(&[good, bad, bad], &t).unwrap();
        assert_eq!(d.at_window, 2);
        assert!(d.message.contains("two consecutive windows"));
    }

    #[test]
    fn a_thin_window_neither_breaks_nor_extends_a_run() {
        let t = DriftTolerance::default();
        let bad = w(0.7, 0.8, 100);
        let thin = w(0.99, 0.1, 5);
        assert!(coverage_drift(&[bad, thin, bad], &t).is_some());
        assert!(coverage_drift(&[thin, thin, bad], &t).is_none());
    }

    #[test]
    fn the_cost_ladder_passes_only_if_the_strategy_still_earns_at_the_required_multiplier() {
        let ladder = [(1.0, 0.8), (2.0, 0.4), (3.0, -0.2), (4.0, -0.8)];
        let v = cost_ladder_verdict(&ladder, 2.0);
        assert!(v.passes);
        assert_eq!(v.last_positive, Some(2.0));
        assert!((v.breakeven.unwrap() - 2.666_666).abs() < 1e-3);
        assert!(!cost_ladder_verdict(&ladder, 3.0).passes);
        assert!(!cost_ladder_verdict(&[], 1.0).passes);
        // A ladder that stops short of the required multiplier cannot pass it.
        assert!(!cost_ladder_verdict(&[(1.0, 0.5)], 2.0).passes);
    }

    #[test]
    fn the_racing_report_says_where_it_is_off_and_why() {
        let on = FidelityCalibration::from_scores("a", "b", &[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 2.0, 3.0, 4.0, 5.0]);
        let off = FidelityCalibration::from_scores("b", "c", &[1.0, 2.0, 3.0, 4.0, 5.0], &[3.0, 1.0, 5.0, 2.0, 4.0]);
        let few = FidelityCalibration::from_scores("c", "d", &[1.0], &[1.0]);
        let r = racing_report(&[on, off, few]);
        assert!(r[0].contains("on"));
        assert!(r[1].contains("off") && r[1].contains("below 0.8"));
        assert!(r[2].contains("too few"));
    }
}
