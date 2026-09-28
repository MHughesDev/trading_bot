//! Macro-series vintages (Set P, P-3.1; DATA-005 §11 S4, §12).
//!
//! A macro series is not one number per date. It is one number per
//! `(series_id, observation_date, vintage_date)`, because statistical agencies
//! revise. FRED serves the *current* value for a historical date — its
//! `realtime_start`/`realtime_end` both default to today — so "GDP for 2023-Q1"
//! comes back as the third estimate whatever date you ask about. ALFRED is the
//! same endpoint with a real-time range, and it returns the value as it stood.
//! That difference is the whole reason S4 names ALFRED and not only FRED.
//!
//! Every read here takes an `as_of` and returns the latest vintage knowable by
//! then. There is no `latest()` and no `current()`, so the wrong answer is
//! unrepresentable rather than merely discouraged.
//!
//! **Knowledge time is a [`Knowledge`], not a timestamp.** ALFRED gives a vintage
//! *date* and no time of day, so a backfilled row's knowledge time has to be
//! derived, and a derived instant and an observed one must stay distinguishable
//! all the way to storage (ADR-P2-24, D-P3). Carrying [`Knowledge`] rather than a
//! `DateTime` plus a `bool` is what makes "we forgot to set the flag"
//! unrepresentable: there is no constructor that takes an instant without also
//! saying where it came from.
//!
//! An absent value is [`None`], never zero. ALFRED writes `"."` for a period an
//! agency has not published, and a zero there is a number a strategy could trade.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::source::{Collection, CostClass, Knowledge, SourceDecl, SourceKind};

impl SourceDecl {
    /// ALFRED's vintage-aware observations endpoint.
    ///
    /// The declared lag is one day less a second, because that is exactly the
    /// derivation this module performs: a vintage published *somewhere* on its
    /// vintage day is treated as knowable at the end of that day. Midnight would
    /// claim the number existed before its release; the end of the day is late,
    /// and late is a cost where early is a lookahead bug.
    ///
    /// A forward collector that watches a release land records
    /// [`Collection::Forward`] instead and keeps its real receipt.
    #[must_use]
    pub const fn alfred_observations() -> Self {
        Self {
            name: "alfred_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(86_399),
            cost_class: CostClass::Free,
            notes: "ALFRED reports a vintage DATE with no time of day; the conservative knowable instant is the end of that day",
        }
    }
}

/// One observation of one macro series, as it stood on one vintage date.
///
/// `observation_date` is the period the number *describes*. `vintage_date` is the
/// day that number was published. They are never the same concept, and conflating
/// them is the macro equivalent of the fundamentals acceptance-time bug
/// ([`crate::fundamentals`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroObservation {
    /// FRED/ALFRED series id, e.g. `GDPC1`, `CPIAUCSL`, `DFF`.
    pub series_id: String,
    /// The period the value describes (ALFRED's `date`).
    pub observation_date: NaiveDate,
    /// The day this value became the published value (ALFRED's `realtime_start`).
    pub vintage_date: NaiveDate,
    /// `None` where ALFRED reports `"."` — missing, not zero.
    pub value: Option<Decimal>,
    /// When the platform could first have known this vintage, and whether that
    /// instant was observed or inferred.
    pub knowledge: Knowledge,
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

/// Midnight UTC on `date`.
fn start_of(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight exists"))
}

/// The conservative knowledge stamp for a vintage published on `vintage_date`.
///
/// Releases land during the day — 08:30 US/Eastern for most BLS and BEA series —
/// and the exact minute is not in ALFRED, so this is [`Knowledge::Derived`] and
/// carries [`crate::QualityFlags::BACKFILLED_KNOWLEDGE_TIME`] wherever it is
/// written.
#[must_use]
pub fn derived_knowledge(vintage_date: NaiveDate) -> Knowledge {
    SourceDecl::alfred_observations()
        .knowledge_for(start_of(vintage_date), Collection::Historical)
        .expect("a historical pull cannot precede its own event time")
}

impl MacroObservation {
    /// An observation whose knowledge time is derived from its vintage day.
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
            knowledge: derived_knowledge(vintage_date),
        }
    }

    /// An observation the platform watched arrive — a forward poll that caught a
    /// release on the day. Its receipt is kept, so it is forward evidence (D-P3).
    ///
    /// # Errors
    /// [`VintageError::KnowledgeBeforeVintage`] if the receipt predates the
    /// vintage day, which would be a clock or parsing fault rather than a fact.
    pub fn observed(
        series_id: impl Into<String>,
        observation_date: NaiveDate,
        vintage_date: NaiveDate,
        value: Option<Decimal>,
        received_at: DateTime<Utc>,
    ) -> Result<Self, VintageError> {
        let row = Self {
            series_id: series_id.into(),
            observation_date,
            vintage_date,
            value,
            knowledge: Knowledge::Observed { at: received_at },
        };
        row.validate()?;
        Ok(row)
    }

    /// When the platform could first have used this figure.
    #[must_use]
    pub const fn knowledge_time(&self) -> DateTime<Utc> {
        self.knowledge.time()
    }

    /// # Errors
    /// Rejects rows whose vintage precedes the period they describe, and rows
    /// whose knowledge time precedes their vintage day.
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
        if self.knowledge_time() < start_of(self.vintage_date) {
            return Err(VintageError::KnowledgeBeforeVintage {
                series_id: self.series_id.clone(),
                vintage_date: self.vintage_date,
                knowledge_time: self.knowledge_time(),
            });
        }
        Ok(())
    }
}

/// The value of `series_id` for `observation_date` as it stood at `as_of`.
///
/// Among the rows knowable by `as_of`, the one with the greatest `vintage_date`;
/// ties broken by the later knowledge time. `None` when the series had published
/// nothing about that period by `as_of` — the correct answer, not an error, and
/// what a warm-up window exists to absorb.
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
                && r.knowledge_time() <= as_of
        })
        .max_by(|a, b| {
            a.vintage_date
                .cmp(&b.vintage_date)
                .then(a.knowledge_time().cmp(&b.knowledge_time()))
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
        .filter(|r| r.series_id == series_id && r.knowledge_time() <= as_of)
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
    use crate::QualityFlags;
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
        let err = MacroObservation::observed(
            "GDPC1",
            d(2023, 1, 1),
            d(2023, 4, 27),
            Some(dec!(1.0)),
            t(2023, 4, 26),
        )
        .expect_err("a receipt before the vintage day is not a fact");
        assert!(matches!(err, VintageError::KnowledgeBeforeVintage { .. }));
    }

    #[test]
    fn the_derived_knowledge_time_is_the_end_of_the_vintage_day() {
        let obs = MacroObservation::derived("DFF", d(2024, 1, 1), d(2024, 1, 2), Some(dec!(5.33)));
        assert!(obs.knowledge.is_derived());
        assert_eq!(obs.knowledge_time().date_naive(), d(2024, 1, 2));
        // Not midnight: midnight would claim the number existed before its release.
        assert!(obs.knowledge_time().time().hour() > 0);
        assert_eq!(obs.validate(), Ok(()));
    }

    /// D-P3: a derived vintage is never forward evidence and always carries the
    /// backfill flag, while a vintage the platform watched land is both observed
    /// and unflagged. The two must never collapse into each other.
    #[test]
    fn derived_and_observed_vintages_stay_distinguishable() {
        let backfilled =
            MacroObservation::derived("DFF", d(2024, 1, 1), d(2024, 1, 2), Some(dec!(5.33)));
        assert!(!backfilled.knowledge.is_forward_evidence());
        assert!(backfilled
            .knowledge
            .quality_flags()
            .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));

        let watched = MacroObservation::observed(
            "DFF",
            d(2024, 1, 1),
            d(2024, 1, 2),
            Some(dec!(5.33)),
            Utc.from_utc_datetime(&d(2024, 1, 2).and_hms_opt(13, 31, 2).expect("valid")),
        )
        .expect("a receipt on the vintage day is a fact");
        assert!(watched.knowledge.is_forward_evidence());
        assert_eq!(watched.knowledge.quality_flags(), QualityFlags::NONE);
        assert_ne!(backfilled.knowledge, watched.knowledge);
    }

    /// An absent measurement is never a zero: a `None` value is carried as `None`
    /// through every read, and never compares equal to a published zero.
    #[test]
    fn a_missing_value_is_not_a_zero() {
        let rows = vec![
            MacroObservation::derived("CPIAUCSL", d(2024, 1, 1), d(2024, 2, 13), None),
            MacroObservation::derived("CPIAUCSL", d(2024, 2, 1), d(2024, 3, 12), Some(dec!(0))),
        ];
        let missing = value_as_of(&rows, "CPIAUCSL", d(2024, 1, 1), t(2024, 4, 1))
            .expect("the row exists even though the figure does not");
        assert_eq!(missing.value, None);
        let published_zero = value_as_of(&rows, "CPIAUCSL", d(2024, 2, 1), t(2024, 4, 1))
            .expect("a published zero is a figure");
        assert_eq!(published_zero.value, Some(dec!(0)));
        assert_ne!(missing.value, published_zero.value);
    }
}
