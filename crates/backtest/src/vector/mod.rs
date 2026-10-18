//! Tier 0: a vectorised screen that can never judge (Set M M-3.2; BT-09; FEAT-005 §7, B7).
//!
//! Screening many candidates cheaply is worth having only if it ranks them the way the real
//! engine would, and only if its numbers can never be mistaken for the real engine's. Both are
//! enforced here as types, not as conventions:
//!
//! * **Off until calibrated.** [`Tier0::enable`] is the only constructor. It takes the
//!   calibration row (Spearman rho of the screen's scores against the event engine's, on a
//!   calibration set) and refuses without one, refuses below [`MIN_RHO`] saying why, and refuses
//!   a calibration over too few pairs to mean anything. There is no `Default` and no `new`.
//! * **Never a verdict.** [`ScreenResult`] is a different type from `RunResult`, has no
//!   conversion to it, and carries no `run_id`, no ticket and no flags, so nothing that takes a
//!   `RunResult` (a gate, the ledger's outcome, the trial store) can be handed one. The compile
//!   failure is checked by the doc test below.
//!
//! ```compile_fail
//! use backtest::run::RunResult;
//! use backtest::vector::ScreenResult;
//! fn feed_a_gate(s: ScreenResult) -> RunResult {
//!     s.into() // a screening result is not a run result, and there is no conversion
//! }
//! ```
//!
//! The evaluator is the position-series one (decide on close, fill at the next open, fee plus
//! half-spread): FEAT-005's tier 0 is "bar-close signals, next-open fills, no intrabar exits",
//! which is exactly what that evaluator already is. What it adds here is the calibration gate
//! and the type wall.

use chrono::{DateTime, Utc};

use crate::position_series::{evaluate, Bar, EvalConfig, PositionSeries};

/// The calibration bar: below this the screen does not rank like the engine.
pub const MIN_RHO: f64 = 0.8;
/// Fewer pairs than this and a rank correlation is an anecdote.
pub const MIN_PAIRS: usize = 20;

/// The calibration row.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub spearman_rho: f64,
    pub n_pairs: usize,
    pub measured_at: DateTime<Utc>,
    /// Which event engine it was measured against.
    pub engine: String,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Tier0Off {
    #[error("tier 0 is off: no calibration row exists. Run the screen and the event engine on one calibration set and record their rank correlation")]
    NoCalibration,
    #[error("tier 0 is off: its rank correlation with the event engine is {rho:.3}, below the {needed} it needs, so it does not order candidates the way the engine does")]
    BelowThreshold { rho: f64, needed: f64 },
    #[error("tier 0 is off: the calibration is over {n} pairs and needs at least {MIN_PAIRS}")]
    TooFewPairs { n: usize },
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum CalibrationError {
    #[error("the two score lists differ in length ({0} vs {1})")]
    LengthMismatch(usize, usize),
    #[error("a score is not finite")]
    NotFinite,
}

#[allow(clippy::float_cmp)] // exact ties define average ranks
fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = (i + j) as f64 / 2.0 + 1.0; // ties share their average rank
        for k in i..=j {
            r[idx[k]] = avg;
        }
        i = j + 1;
    }
    r
}

/// Spearman rank correlation. `None` when either side has no variance in rank.
#[must_use]
pub fn spearman(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.len() != b.len() || a.len() < 2 {
        return None;
    }
    let (ra, rb) = (ranks(a), ranks(b));
    #[allow(clippy::cast_precision_loss)]
    let n = ra.len() as f64;
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in ra.iter().zip(&rb) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    (saa > 0.0 && sbb > 0.0).then(|| sab / (saa * sbb).sqrt())
}

/// Measure the calibration: the screen's scores and the engine's, candidate for candidate.
///
/// # Errors
/// Lists of different lengths or a non-finite score.
pub fn calibrate(
    screen_scores: &[f64],
    engine_scores: &[f64],
    engine: &str,
    now: DateTime<Utc>,
) -> Result<Calibration, CalibrationError> {
    if screen_scores.len() != engine_scores.len() {
        return Err(CalibrationError::LengthMismatch(screen_scores.len(), engine_scores.len()));
    }
    if screen_scores.iter().chain(engine_scores).any(|x| !x.is_finite()) {
        return Err(CalibrationError::NotFinite);
    }
    Ok(Calibration {
        // A constant score list has no ranking to agree with: its correlation is 0, not 1.
        spearman_rho: spearman(screen_scores, engine_scores).unwrap_or(0.0),
        n_pairs: screen_scores.len(),
        measured_at: now,
        engine: engine.to_string(),
    })
}

/// A screening result. **Not a `RunResult`** and not convertible to one.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScreenResult {
    pub total_return: f64,
    pub n_fills: usize,
}

/// The enabled screen. Constructible only through [`Tier0::enable`].
#[derive(Clone, Debug)]
pub struct Tier0 {
    calibration: Calibration,
}

impl Tier0 {
    /// # Errors
    /// No calibration, one below [`MIN_RHO`], or one over too few pairs: the reason, named.
    pub fn enable(calibration: Option<&Calibration>) -> Result<Self, Tier0Off> {
        let c = calibration.ok_or(Tier0Off::NoCalibration)?;
        if c.n_pairs < MIN_PAIRS {
            return Err(Tier0Off::TooFewPairs { n: c.n_pairs });
        }
        if c.spearman_rho < MIN_RHO {
            return Err(Tier0Off::BelowThreshold { rho: c.spearman_rho, needed: MIN_RHO });
        }
        Ok(Self { calibration: c.clone() })
    }

    #[must_use]
    pub fn calibration(&self) -> &Calibration {
        &self.calibration
    }

    /// Screen candidates over one set of bars. A series the evaluator refuses scores `None`.
    #[must_use]
    pub fn screen(&self, series: &[PositionSeries], bars: &[Bar], cfg: &EvalConfig) -> Vec<Option<ScreenResult>> {
        series
            .iter()
            .map(|s| {
                evaluate(s, bars, cfg).ok().map(|run| ScreenResult {
                    total_return: run.final_equity / cfg.initial_equity - 1.0,
                    n_fills: run.fills.len(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_series::{Costs, PositionPoint, PositionUnits};
    use chrono::TimeZone;

    fn cal(rho: f64, n: usize) -> Calibration {
        Calibration { spearman_rho: rho, n_pairs: n, measured_at: Utc.with_ymd_and_hms(2026, 10, 4, 0, 0, 0).unwrap(), engine: "event".into() }
    }

    #[test]
    fn tier_0_is_off_until_a_calibration_row_exists_and_says_so() {
        assert_eq!(Tier0::enable(None).unwrap_err(), Tier0Off::NoCalibration);
        assert!(Tier0::enable(None).unwrap_err().to_string().contains("no calibration row"));
    }

    #[test]
    fn a_calibration_below_0_8_keeps_it_off_and_says_why() {
        let e = Tier0::enable(Some(&cal(0.79, 100))).unwrap_err();
        assert!(matches!(e, Tier0Off::BelowThreshold { .. }));
        assert!(e.to_string().contains("0.790") && e.to_string().contains("0.8"), "{e}");
        assert!(Tier0::enable(Some(&cal(0.8, 100))).is_ok(), "the bar is inclusive");
        assert!(matches!(Tier0::enable(Some(&cal(0.95, 5))), Err(Tier0Off::TooFewPairs { n: 5 })));
    }

    #[allow(clippy::float_cmp)] // exact constants
    #[test]
    fn spearman_is_one_for_a_monotone_relation_minus_one_for_reversed_and_ties_share_rank() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((spearman(&a, &[10.0, 20.0, 35.0, 80.0, 90.0]).unwrap() - 1.0).abs() < 1e-12);
        assert!((spearman(&a, &[5.0, 4.0, 3.0, 2.0, 1.0]).unwrap() + 1.0).abs() < 1e-12);
        assert!(spearman(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]).is_none(), "no variance in rank, no correlation");
        let calibrated = calibrate(&a, &[1.0, 2.0, 3.0, 4.0, 5.0], "event", Utc::now()).unwrap();
        assert!((calibrated.spearman_rho - 1.0).abs() < 1e-12);
        assert!(calibrate(&a, &[1.0], "event", Utc::now()).is_err());
        assert!(calibrate(&[f64::NAN, 1.0], &[1.0, 2.0], "event", Utc::now()).is_err());
        assert_eq!(calibrate(&[1.0; 30], &[1.0; 30], "event", Utc::now()).unwrap().spearman_rho, 0.0);
    }

    #[test]
    fn an_enabled_screen_scores_candidates_and_a_refused_series_scores_none() {
        let bars: Vec<Bar> = (0..60)
            .map(|i| {
                let p = 100.0 + f64::from(i) * 0.3;
                Bar { ts_ns: (i64::from(i) + 1) * 60_000_000_000, open: p, close: p + 0.1 }
            })
            .collect();
        let long: Vec<PositionPoint> = bars.iter().map(|b| PositionPoint::new(b.ts_ns, 1.0)).collect();
        let good = PositionSeries { points: long, position_units: PositionUnits::Fraction, allow_short: false };
        let off_grid = PositionSeries { points: vec![PositionPoint::new(7, 1.0)], position_units: PositionUnits::Fraction, allow_short: false };
        let cfg = EvalConfig { initial_equity: 10_000.0, costs: Costs { fee_bps: 5.0, half_spread_bps: 1.0 }, lot_size: None };
        let tier0 = Tier0::enable(Some(&cal(0.9, 50))).unwrap();
        let out = tier0.screen(&[good, off_grid], &bars, &cfg);
        assert!(out[0].as_ref().unwrap().total_return > 0.0, "a long series in a rising market earns");
        assert!(out[1].is_none(), "a series not on the decision times is refused, not scored");
        assert_eq!(tier0.calibration().engine, "event");
    }
}
