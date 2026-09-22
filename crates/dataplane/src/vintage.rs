//! Macro-series vintages (Set P, P-3.1; DATA-005 §11 S4, §12).
//!
//! A macro series is not one number per date. It is one number per
//! `(observation_date, vintage_date)` pair, because statistical agencies revise.
//! FRED serves the *current* value for a historical date; ALFRED serves the value
//! as it stood on a chosen day. That difference is the whole reason S4 names
//! ALFRED, and it is why nothing in this module can answer a question without
//! being told an `as_of`.
//!
//! The naive read — "GDP for 2023-Q1" — returns today's revised figure, which no
//! strategy could have traded on in April 2023. Every read here takes an `as_of`
//! and returns the latest vintage published on or before it. There is no
//! `latest()` and no `current()`, so the wrong answer is unrepresentable rather
//! than merely discouraged.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// One observation of one macro series, as it stood on one vintage date.
///
/// `observation_date` is the period the number *describes*. `vintage_date` is the
/// day that number was published. They are never the same concept and conflating
/// them is the macro equivalent of the fundamentals acceptance-time bug
/// ([`crate::fundamentals`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroObservation {
    /// FRED/ALFRED series id, e.g. `GDPC1`, `CPIAUCSL`, `DFF`.
    pub series_id: String,
    /// The period the value describes (ALFRED's `date`).
    pub observation_date: NaiveDate,
    /// The day this value was first published (ALFRED's `realtime_start`).
    pub vintage_date: NaiveDate,
    /// `None` where ALFRED reports `"."` — missing, not zero.
    pub value: Option<Decimal>,
    /// When the platform could first have known this vintage.
    pub knowledge_time: DateTime<Utc>,
    /// True when `knowledge_time` was derived from `vintage_date` rather than
    /// observed at ingestion (ADR-P2-24's `BACKFILLED_KNOWLEDGE_TIME`).
    pub knowledge_time_derived: bool,
}

/// Why a row is not a legitimate observation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VintageError {
    /// A value published before the period it measures began.
    #[error(
        "series {series_id}: vintage {vintage_date} precedes observation date \
         {observation_date} — a value cannot be published before the period it measures"
    )]
    VintagePrecedesObservation {
        series_id: String,
        observation_date: NaiveDate,
        vintage_date: NaiveDate,
    },
    /// A knowledge time earlier than the vintage day it was published on.
    #[error(
        "series {series_id}: knowledge_time {knowledge_time} precedes vintage date \
         {vintage_date} — the platform cannot know a value before it is published"
    )]
    KnowledgeBeforeVintage {
        series_id: String,
        vintage_date: NaiveDate,
        knowledge_time: DateTime<Utc>,
    },
    #[error("series id must not be empty")]
    EmptySeriesId,
}

/// The conservative knowledge time for a vintage published on `vintage_date`.
///
/// Releases land during the day — 08:30 US/Eastern for most BLS and BEA series —
/// and the exact minute is not in ALFRED. Midnight would claim the number was
/// knowable before it existed, so the derived time is the *end* of the vintage
/// day. Being late is a cost; being early is a lookahead bug.
#[must_use]
pub fn derived_knowledge_time(vintage_date: NaiveDate) -> DateTime<Utc> {
    let end = vintage_date
        .and_hms_opt(23, 59, 59)
        .unwrap_or_else(|| vintage_date.and_hms_opt(0, 0, 0).expect("midnight exists"));
    Utc.from_utc_datetime(&end)
}

impl MacroObservation {
    /// Builds an observation whose knowledge time is derived from its vintage day.
    #[must_use]
    pub fn derived(
        series_id: impl Into<String>,
        observation_date: NaiveDate,
        vintage_date: NaiveDate,
        value: Option<Decimal>,
    ) -> Self {
        Self {
            series_id: series_id.into(),
            observation_date,
            vintage_date,
            value,
            knowledge_time: derived_knowledge_time(vintage_date),
            knowledge_time_derived: true,
        }
    }

    /// # Errors
    /// Rejects rows whose vintage precedes the period they describe, or whose
    /// knowledge time precedes their vintage day.
    pub fn validate(&self) -> Result<(), VintageError> {
        if self.series_id.trim().is_empty() {
            return Err(VintageError::EmptySeriesId);
        }
        if self.vintage_date < self.observation_date {
            return Err(VintageError::VintagePrecedesObservation {
                series_id: self.series_id.clone(),
                observation_date: self.observation_date,
                vintage_date: self.vintage_date,
            });
        }
        let vintage_start = Utc.from_utc_datetime(
            &self
                .vintage_date
                .and_hms_opt(0, 0, 0)
                .expect("midnight exists"),
        );
        if self.knowledge_time < vintage_start {
            return Err(VintageError::KnowledgeBeforeVintage {
                series_id: self.series_id.clone(),
                vintage_date: self.vintage_date,
                knowledge_time: self.knowledge_time,
            });
        }
        Ok(())
    }
}

/// The value of `series_id` for `observation_date` as it stood at `as_of`.
///
/// Among the rows whose `knowledge_time <= as_of`, the one with the greatest
/// `vintage_date`; ties broken by the later knowledge time. `None` when the
/// series had published nothing about that period by `as_of` — which is the
/// correct answer, not an error, and is what a warm-up window exists to absorb.
#[must_use]
pub fn value_as_of<'a>(
    rows: &'a [MacroObservation],
    series_id: &str,
    observation_date: NaiveDate,
    as_of: DateTime<Utc>,
) -> Option<&'a MacroObservation> {
    rows.iter()
        .filter(|r| {
            r.series_id == series_id
                && r.observation_date == observation_date
                && r.knowledge_time <= as_of
        })
        .max_by(|a, b| {
            a.vintage_date
                .cmp(&b.vintage_date)
                .then(a.knowledge_time.cmp(&b.knowledge_time))
        })
}

/// The whole series as it stood at `as_of`: one row per observation date, each
/// the latest vintage knowable by then, ordered by observation date.
#[must_use]
pub fn series_as_of<'a>(
    rows: &'a [MacroObservation],
    series_id: &str,
    as_of: DateTime<Utc>,
) -> Vec<&'a MacroObservation> {
    let mut dates: Vec<NaiveDate> = rows
        .iter()
        .filter(|r| r.series_id == series_id && r.knowledge_time <= as_of)
        .map(|r| r.observation_date)
        .collect();
    dates.sort_unstable();
    dates.dedup();
    dates
        .into_iter()
        .filter_map(|d| value_as_of(rows, series_id, d, as_of))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
    }

    fn t(y: i32, m: u32, day: u32) -> DateTime<Utc> {
        Utc.from_utc_datetime(&d(y, m, day).and_hms_opt(12, 0, 0).expect("noon exists"))
    }

    /// A series whose Q1 print is revised in June.
    fn revised_series() -> Vec<MacroObservation> {
        vec![
            MacroObservation::derived("GDPC1", d(2023, 1, 1), d(2023, 4, 27), Some(dec!(2.0))),
            MacroObservation::derived("GDPC1", d(2023, 1, 1), d(2023, 6, 29), Some(dec!(2.6))),
        ]
    }

    /// P-3.1 acceptance, the adversarial half: a revision is planted, and the
    /// as-of read before it must still return the ORIGINAL value. A store that
    /// returned 2.6 here would make every backtest of April 2023 fiction.
    #[test]
    fn a_planted_revision_does_not_leak_backwards() {
        let rows = revised_series();
        let before = value_as_of(&rows, "GDPC1", d(2023, 1, 1), t(2023, 5, 15))
            .expect("the April print was knowable in May");
        assert_eq!(before.value, Some(dec!(2.0)));
        assert_eq!(before.vintage_date, d(2023, 4, 27));
    }

    #[test]
    fn the_revision_is_returned_once_it_is_published() {
        let rows = revised_series();
        let after = value_as_of(&rows, "GDPC1", d(2023, 1, 1), t(2023, 7, 15))
            .expect("the June revision was knowable in July");
        assert_eq!(after.value, Some(dec!(2.6)));
    }

    #[test]
    fn nothing_is_returned_before_the_first_publication() {
        let rows = revised_series();
        assert!(value_as_of(&rows, "GDPC1", d(2023, 1, 1), t(2023, 4, 1)).is_none());
    }

    /// The same query run twice returns the same answer (P-3.1 acceptance).
    #[test]
    fn the_read_is_deterministic() {
        let rows = revised_series();
        let a = value_as_of(&rows, "GDPC1", d(2023, 1, 1), t(2023, 5, 15));
        let b = value_as_of(&rows, "GDPC1", d(2023, 1, 1), t(2023, 5, 15));
        assert_eq!(a, b);
    }

    #[test]
    fn series_as_of_takes_the_latest_vintage_per_date() {
        let mut rows = revised_series();
        rows.push(MacroObservation::derived(
            "GDPC1",
            d(2023, 4, 1),
            d(2023, 7, 27),
            Some(dec!(2.1)),
        ));
        let at_august = series_as_of(&rows, "GDPC1", t(2023, 8, 1));
        assert_eq!(at_august.len(), 2);
        assert_eq!(at_august[0].value, Some(dec!(2.6)));
        assert_eq!(at_august[1].value, Some(dec!(2.1)));

        // In May only the first period had been published, at its first vintage.
        let at_may = series_as_of(&rows, "GDPC1", t(2023, 5, 15));
        assert_eq!(at_may.len(), 1);
        assert_eq!(at_may[0].value, Some(dec!(2.0)));
    }

    #[test]
    fn a_vintage_before_its_observation_period_is_refused() {
        let bad = MacroObservation::derived("GDPC1", d(2023, 4, 1), d(2023, 1, 1), Some(dec!(1.0)));
        assert!(matches!(
            bad.validate(),
            Err(VintageError::VintagePrecedesObservation { .. })
        ));
    }

    #[test]
    fn a_knowledge_time_before_its_vintage_day_is_refused() {
        let mut bad =
            MacroObservation::derived("GDPC1", d(2023, 1, 1), d(2023, 4, 27), Some(dec!(1.0)));
        bad.knowledge_time = t(2023, 4, 26);
        assert!(matches!(
            bad.validate(),
            Err(VintageError::KnowledgeBeforeVintage { .. })
        ));
    }

    #[test]
    fn the_derived_knowledge_time_is_the_end_of_the_vintage_day() {
        let obs = MacroObservation::derived("DFF", d(2024, 1, 1), d(2024, 1, 2), Some(dec!(5.33)));
        assert!(obs.knowledge_time_derived);
        assert_eq!(obs.knowledge_time.date_naive(), d(2024, 1, 2));
        // Not midnight: midnight would claim the number existed before its release.
        assert!(obs.knowledge_time.time().hour() > 0);
        assert_eq!(obs.validate(), Ok(()));
    }
}
