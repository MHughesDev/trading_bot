//! Annualisation from the instrument calendar (Set M M-2.6; FEAT-005 §8).
//!
//! A 24/7 crypto instrument has 525,600 one-minute periods a year and 8,760 hourly ones; an
//! equity session has far fewer. `periods_per_year` takes the **resolution and the calendar**,
//! so a 1m run and a 1h run of one series annualise by their own counts. A ratio built from
//! noisy short bars (Sortino, Calmar at 5m or finer) is annualised across so many periods that
//! the figure is mostly a statement about the period count, so [`warnings`] says so and
//! [`default_objective`] steers fine resolutions to expectancy or profit factor instead.

use crate::run::EvalResolution;

/// What the instrument trades on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calendar {
    /// 24 hours, 7 days (crypto, prediction markets).
    Continuous,
    /// A regular equity session: 6.5 hours, 252 days.
    EquitySession,
}

/// Periods in a year at this resolution on this calendar.
#[must_use]
pub fn periods_per_year(res: EvalResolution, cal: Calendar) -> f64 {
    let minutes = match res {
        EvalResolution::Min1 => 1.0,
        EvalResolution::Min5 => 5.0,
        EvalResolution::Min10 => 10.0,
        EvalResolution::Min15 => 15.0,
        EvalResolution::Min30 => 30.0,
        EvalResolution::Hour1 => 60.0,
        EvalResolution::Day1 => {
            return if cal == Calendar::Continuous {
                365.0
            } else {
                252.0
            }
        }
    };
    let minutes_per_year = match cal {
        Calendar::Continuous => 365.0 * 24.0 * 60.0,
        Calendar::EquitySession => 252.0 * 6.5 * 60.0,
    };
    minutes_per_year / minutes
}

/// Whether a resolution is fine enough that downside-deviation ratios are mostly noise.
#[must_use]
pub fn is_fine(res: EvalResolution) -> bool {
    matches!(res, EvalResolution::Min1 | EvalResolution::Min5)
}

/// The metric an objective defaults to at this resolution.
#[must_use]
pub fn default_objective(res: EvalResolution) -> &'static str {
    if is_fine(res) {
        "expectancy"
    } else {
        "sharpe"
    }
}

/// Warnings a report must carry for an annualised metric at this resolution.
#[must_use]
pub fn warnings(metric: &str, res: EvalResolution) -> Vec<String> {
    let mut w = Vec::new();
    if is_fine(res) && matches!(metric, "sortino" | "calmar") {
        w.push(format!(
            "{metric} annualised from {res:?} bars scales a very short-horizon statistic by sqrt of \
             hundreds of thousands of periods; read expectancy or profit factor, or aggregate to a \
             coarser resolution first"
        ));
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::float_cmp)] // exact constants
    #[test]
    fn one_minute_and_one_hour_runs_annualise_by_their_own_period_counts() {
        assert_eq!(
            periods_per_year(EvalResolution::Min1, Calendar::Continuous),
            525_600.0
        );
        assert_eq!(
            periods_per_year(EvalResolution::Hour1, Calendar::Continuous),
            8_760.0
        );
        assert_eq!(
            periods_per_year(EvalResolution::Day1, Calendar::Continuous),
            365.0
        );
        assert_eq!(
            periods_per_year(EvalResolution::Day1, Calendar::EquitySession),
            252.0
        );
        assert!(
            (periods_per_year(EvalResolution::Hour1, Calendar::EquitySession) - 252.0 * 6.5).abs()
                < 1e-9
        );
    }

    #[test]
    fn a_one_minute_sortino_carries_a_warning_and_an_hourly_one_does_not() {
        assert_eq!(warnings("sortino", EvalResolution::Min1).len(), 1);
        assert_eq!(warnings("calmar", EvalResolution::Min5).len(), 1);
        assert!(warnings("sortino", EvalResolution::Hour1).is_empty());
        assert!(warnings("sharpe", EvalResolution::Min1).is_empty());
    }

    #[test]
    fn fine_resolutions_default_to_expectancy() {
        assert_eq!(default_objective(EvalResolution::Min1), "expectancy");
        assert_eq!(default_objective(EvalResolution::Min15), "sharpe");
    }
}
