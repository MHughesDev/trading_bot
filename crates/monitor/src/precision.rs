//! Scoring the monitor itself (COMP-007 §4.1).
//!
//! The platform measures its strategies; it does not get to exempt its own
//! instruments from measurement. Every breach is labelled afterwards — by the human
//! who resumed, or automatically when a scaled-down strategy is restored to full
//! with no intervening change — and precision per metric family is published
//! monthly beside the strategy scorecards.
//!
//! A family whose precision falls below 0.5 has its limits re-derived from data. Not
//! "is reviewed": more than half its alarms were wrong, and an operator who has
//! learned that is right to ignore it.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::family::Family;

/// The precision at which a family is flagged for re-derivation.
pub const PRECISION_FLOOR: f64 = 0.5;

/// What a breach turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrecisionLabel {
    /// Something really was wrong.
    TruePositive,
    /// The alarm was noise.
    FalsePositive,
}

impl PrecisionLabel {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TruePositive => "true_positive",
            Self::FalsePositive => "false_positive",
        }
    }
}

/// A family's score for one month.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FamilyPrecision {
    pub period_month: NaiveDate,
    pub family: Family,
    pub true_positives: u32,
    pub false_positives: u32,
    /// `None` when nothing was labelled. Not zero, and not one.
    pub precision: Option<f64>,
    pub flagged: bool,
}

impl FamilyPrecision {
    /// Compute a family's score.
    ///
    /// An unlabelled family scores `None` and is **not** flagged. Scoring it zero
    /// would flag every family the month a new one is added; scoring it one would
    /// clear a family nobody has checked. Both are claims the data cannot support,
    /// and "not measured" is the honest third answer.
    #[must_use]
    pub fn compute(
        period_month: NaiveDate,
        family: Family,
        true_positives: u32,
        false_positives: u32,
    ) -> Self {
        let total = true_positives + false_positives;
        let precision = if total == 0 {
            None
        } else {
            Some(f64::from(true_positives) / f64::from(total))
        };
        Self {
            period_month,
            family,
            true_positives,
            false_positives,
            precision,
            flagged: precision.is_some_and(|p| p < PRECISION_FLOOR),
        }
    }

    /// Whether this family's limits should be re-derived from data.
    #[must_use]
    pub fn needs_rederivation(&self) -> bool {
        self.flagged
    }
}

/// Score every family for a month, so the report cannot quietly omit one.
#[must_use]
pub fn report(period_month: NaiveDate, counts: &[(Family, u32, u32)]) -> Vec<FamilyPrecision> {
    Family::all()
        .into_iter()
        .map(|family| {
            let (tp, fp) = counts
                .iter()
                .find(|(f, _, _)| *f == family)
                .map_or((0, 0), |(_, tp, fp)| (*tp, *fp));
            FamilyPrecision::compute(period_month, family, tp, fp)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn month() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 1).expect("date")
    }

    #[test]
    fn precision_is_true_over_total() {
        let p = FamilyPrecision::compute(month(), Family::Execution, 3, 1);
        assert_eq!(p.precision, Some(0.75));
        assert!(!p.flagged);
    }

    #[test]
    fn a_family_below_half_is_flagged_for_rederivation() {
        // COMP-007 §8.10.
        let p = FamilyPrecision::compute(month(), Family::Behaviour, 2, 3);
        assert_eq!(p.precision, Some(0.4));
        assert!(p.flagged);
        assert!(p.needs_rederivation());
    }

    #[test]
    fn exactly_half_is_not_flagged() {
        // The floor is "below 0.5", and a boundary that flags at exactly half
        // would re-derive a family that is right as often as it is wrong — which
        // is bad, but is not what the spec says to act on.
        let p = FamilyPrecision::compute(month(), Family::Data, 1, 1);
        assert_eq!(p.precision, Some(0.5));
        assert!(!p.flagged);
    }

    #[test]
    fn an_unlabelled_family_is_unmeasured_rather_than_perfect_or_terrible() {
        // The adversarial one, both ways. Zero would flag every new family in its
        // first month; one would clear a family nobody has ever checked — and the
        // second is the dangerous direction, because it is silent.
        let p = FamilyPrecision::compute(month(), Family::Venue, 0, 0);
        assert_eq!(p.precision, None);
        assert!(!p.flagged, "an unmeasured family must not be flagged");
        assert_ne!(
            p.precision,
            Some(1.0),
            "and must not be reported as perfect"
        );
    }

    #[test]
    fn a_family_that_is_always_wrong_scores_zero_and_is_flagged() {
        let p = FamilyPrecision::compute(month(), Family::Pnl, 0, 7);
        assert_eq!(p.precision, Some(0.0));
        assert!(p.flagged);
    }

    #[test]
    fn the_report_covers_every_family_including_the_silent_ones() {
        // A report that lists only the families that fired looks, month to month,
        // like the quiet families do not exist.
        let rows = report(month(), &[(Family::Execution, 4, 0)]);
        assert_eq!(rows.len(), Family::all().len());
        let listed: Vec<Family> = rows.iter().map(|r| r.family).collect();
        assert_eq!(listed, Family::all().to_vec());
        let venue = rows
            .iter()
            .find(|r| r.family == Family::Venue)
            .expect("venue");
        assert_eq!(venue.precision, None);
    }
}
