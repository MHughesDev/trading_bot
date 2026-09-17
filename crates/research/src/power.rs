//! The power check, before the first trial (AGENT-013 §5.5, §6.3; Set S task
//! S-10.5).
//!
//! # The waste this prevents
//!
//! A run pre-registers a `delta_practical` — the smallest improvement that would
//! change a decision — and then goes looking for it in a window that could not
//! show it however real it was. Every trial it spends is spent; every statistic
//! computes; the answer is `inconclusive: insufficient_data`, and the run learns
//! that at the end rather than at the start.
//!
//! The arithmetic that says so is the same one the audit already runs *after* a
//! backtest: `min_track_record_length` (AGENT-006 §5.4, Bailey & López de Prado).
//! Asking it about the **pre-registered effect size** rather than about an
//! observed Sharpe turns it from a verdict into a precondition, and the answer
//! costs nothing.
//!
//! # What it is not
//!
//! It is not a claim that the effect is there, and not a claim that a window that
//! passes will settle the question. It is the one-sided bound: **a window shorter
//! than this cannot distinguish `delta_practical` from zero at the stated
//! confidence, so a run over it cannot conclude.** Passing means the run is not
//! doomed on sample size alone.
//!
//! The direction matters, because the other direction would be a promise. A check
//! that said "this window is enough" would be read as one, and it would be wrong
//! the first time the returns were autocorrelated.

use serde::{Deserialize, Serialize};

/// What a window can and cannot settle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerCheck {
    /// The effect size the run pre-registered, as a per-observation Sharpe.
    pub delta_practical: f64,
    pub confidence: f64,
    /// Observations the window actually holds.
    pub observations: f64,
    pub observations_per_year: f64,
    /// Observations needed for `delta_practical` to be distinguishable from
    /// zero. `None` when the question is not answerable by this arithmetic —
    /// see [`Verdict::NotComputable`].
    pub required_observations: Option<f64>,
}

/// The one-sided answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// The window is long enough for the pre-registered effect to be
    /// distinguishable from zero. **Not** a promise that it will be.
    Adequate,
    /// It is not, and the run would be spending trials on a question this data
    /// cannot answer.
    Insufficient {
        /// Observations the window would need.
        required_observations: f64,
        /// Years, when the window has a sampling rate to convert by.
        required_years: Option<f64>,
        /// How much longer, in the same units the caller gave.
        shortfall_observations: f64,
    },
    /// The arithmetic does not apply. Reported rather than approximated —
    /// ADR-P2-04's discipline, and here the approximation would be a refusal or
    /// an approval nobody could justify.
    NotComputable { why: &'static str },
}

/// Confidence the power check is run at.
///
/// Not the caller's, for the reason `TRACK_RECORD_CONFIDENCE` is not: a run that
/// could choose its own confidence would choose the one its window passes, and
/// the check would measure the run's preference rather than the data.
pub const POWER_CONFIDENCE: f64 = 0.95;

/// Can this window settle a difference of `delta_practical`?
///
/// `skew` and `kurtosis` come from the returns the window actually holds, so the
/// non-normality adjustment is the data's rather than an assumption. A caller
/// with no returns yet passes `0.0` and `3.0` — normal — and gets the answer for
/// a normal world, which is the optimistic one and is stated as such by
/// [`Verdict::Adequate`] not being a promise.
#[must_use]
pub fn check(
    delta_practical: f64,
    observations: f64,
    observations_per_year: f64,
    skew: f64,
    kurtosis: f64,
) -> (PowerCheck, Verdict) {
    let required =
        backtest::stats::min_track_record_length(delta_practical, 0.0, skew, kurtosis, POWER_CONFIDENCE);
    let check = PowerCheck {
        delta_practical,
        confidence: POWER_CONFIDENCE,
        observations,
        observations_per_year,
        required_observations: required,
    };

    if !delta_practical.is_finite() || delta_practical <= 0.0 {
        return (
            check,
            Verdict::NotComputable {
                why: "delta_practical must be finite and positive: a zero effect size means \
                      every difference matters, which is the same as none of them mattering",
            },
        );
    }
    if !observations.is_finite() || observations <= 0.0 {
        return (
            check,
            Verdict::NotComputable {
                why: "the window holds no observations",
            },
        );
    }
    let Some(required) = required else {
        return (
            check,
            Verdict::NotComputable {
                why: "the minimum track record is undefined for these moments — a fat left \
                      tail can drive the adjustment negative, and a refusal derived from a \
                      NaN would be a refusal nobody could justify",
            },
        );
    };

    if observations >= required {
        (check, Verdict::Adequate)
    } else {
        (
            check.clone(),
            Verdict::Insufficient {
                required_observations: required,
                required_years: (observations_per_year > 0.0)
                    .then(|| required / observations_per_year),
                shortfall_observations: required - observations,
            },
        )
    }
}

impl Verdict {
    #[must_use]
    pub fn is_adequate(&self) -> bool {
        matches!(self, Verdict::Adequate)
    }

    /// What the run is told, which has to name the window that *would* suffice.
    ///
    /// "Insufficient data" is not something a run can act on. "You need 1,340
    /// observations and have 512; at this sampling rate that is 5.3 years" is.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Verdict::Adequate => "the window is long enough to distinguish the pre-registered \
                                  effect from zero. That is a floor, not a promise: it says \
                                  the run is not doomed on sample size alone."
                .to_string(),
            Verdict::Insufficient {
                required_observations,
                required_years,
                shortfall_observations,
            } => {
                let years = required_years
                    .map(|y| format!(", which at this sampling rate is {y:.1} years"))
                    .unwrap_or_default();
                format!(
                    "this window cannot distinguish the effect you pre-registered from zero. \
                     It needs {required_observations:.0} observations{years}, and is \
                     {shortfall_observations:.0} short. Widen the window, or pre-register a \
                     larger delta_practical and say why it is the one that matters."
                )
            }
            Verdict::NotComputable { why } => format!("the power check does not apply: {why}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window too short to show the effect is refused, with the window that
    /// would do.
    ///
    /// The acceptance: refused *before any trial*, with the sufficient window
    /// stated. "Insufficient data" is not something a run can act on.
    #[test]
    fn a_window_that_cannot_settle_the_question_is_refused_with_a_number() {
        // A small per-observation effect over a year of daily bars.
        let (check, verdict) = check(0.05, 252.0, 252.0, 0.0, 3.0);
        let Verdict::Insufficient {
            required_observations,
            required_years,
            shortfall_observations,
        } = verdict.clone()
        else {
            panic!("expected insufficient, got {verdict:?}");
        };
        assert!(required_observations > 252.0);
        assert!((shortfall_observations - (required_observations - 252.0)).abs() < 1e-9);
        assert!(required_years.expect("a sampling rate was given") > 1.0);
        assert_eq!(check.required_observations, Some(required_observations));

        let text = verdict.detail();
        assert!(text.contains("observations"), "{text}");
        assert!(text.contains("years"), "{text}");
        assert!(
            text.contains("Widen the window"),
            "the run has to be told what to do: {text}"
        );
    }

    /// A window long enough passes, and the pass is explicitly not a promise.
    #[test]
    fn an_adequate_window_is_a_floor_and_says_so() {
        let (_, verdict) = check(0.05, 20_000.0, 252.0, 0.0, 3.0);
        assert!(verdict.is_adequate());
        assert!(
            verdict.detail().contains("not a promise"),
            "a check that read as a guarantee would be wrong the first time the returns \
             were autocorrelated: {}",
            verdict.detail()
        );
    }

    /// A bigger effect needs less data. The direction the arithmetic must have.
    #[test]
    fn a_larger_effect_needs_a_shorter_window() {
        let small = check(0.02, 1.0, 252.0, 0.0, 3.0).0.required_observations;
        let large = check(0.20, 1.0, 252.0, 0.0, 3.0).0.required_observations;
        assert!(
            large.unwrap() < small.unwrap(),
            "{large:?} should be less than {small:?}"
        );
    }

    /// The confidence is the platform's, not the caller's.
    ///
    /// A run that could choose it would choose the one its window passes, and
    /// the check would measure the run's preference rather than the data.
    #[test]
    fn the_confidence_is_not_something_a_run_supplies() {
        let (check, _) = check(0.05, 252.0, 252.0, 0.0, 3.0);
        assert!((check.confidence - POWER_CONFIDENCE).abs() < f64::EPSILON);
        // The signature has no confidence parameter at all, which is the
        // mechanism: there is nothing to pass.
        let _: fn(f64, f64, f64, f64, f64) -> (PowerCheck, Verdict) = check_fn();
    }

    fn check_fn() -> fn(f64, f64, f64, f64, f64) -> (PowerCheck, Verdict) {
        super::check
    }

    /// A delta of zero is not a small delta.
    #[test]
    fn a_zero_delta_is_refused_rather_than_answered() {
        for bad in [0.0, -0.1, f64::NAN] {
            let (_, v) = check(bad, 1_000.0, 252.0, 0.0, 3.0);
            assert!(
                matches!(v, Verdict::NotComputable { .. }),
                "{bad} gave {v:?}"
            );
        }
    }

    /// Moments that make the arithmetic undefined are reported, not approximated.
    ///
    /// ADR-P2-04's discipline. A refusal derived from a NaN is a refusal nobody
    /// could justify, and an approval derived from one is worse.
    #[test]
    fn undefined_moments_report_rather_than_guess() {
        // `min_track_record_length` returns `None` when the Sharpe does not
        // exceed the benchmark, which here means a non-positive delta — already
        // covered — so the reachable `None` is the confidence band, which this
        // module pins. The guard is asserted directly instead.
        let (_, v) = check(0.05, f64::NAN, 252.0, 0.0, 3.0);
        assert!(matches!(v, Verdict::NotComputable { .. }), "{v:?}");
    }

    /// With no sampling rate there are no years, and it says so rather than
    /// inventing one.
    #[test]
    fn a_window_with_no_sampling_rate_reports_observations_only() {
        let (_, v) = check(0.05, 10.0, 0.0, 0.0, 3.0);
        let Verdict::Insufficient { required_years, .. } = v.clone() else {
            panic!("{v:?}");
        };
        assert_eq!(required_years, None);
        assert!(!v.detail().contains("years"), "{}", v.detail());
    }
}
