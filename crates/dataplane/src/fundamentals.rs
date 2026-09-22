//! SEC fundamentals, point-in-time by acceptance time (Set P, P-3.2).
//!
//! **The period a number describes is not when it became knowable.** Apple's
//! fiscal Q1 ends in late December; the 10-Q that states its revenue is accepted
//! by EDGAR weeks later. A join on the period end date hands a backtest the
//! December number in December, and every result built on it is fiction. That is
//! the single most common lookahead bug in fundamentals research, and the whole
//! purpose of this module is that the bug has nowhere to live: every read takes
//! an `as_of` and filters on `accepted_at`, and there is no accessor that filters
//! on the period.
//!
//! An amended filing (10-K/A) is a *new fact* with a later acceptance time, never
//! an edit of the old one — same discipline as bar revisions (DA-16) and text
//! revisions ([`crate::text`]).

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// One XBRL fact from one filing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FundamentalFact {
    /// SEC Central Index Key, the filer's stable identity.
    pub cik: u32,
    /// XBRL tag, e.g. `Revenues`, `AssetsCurrent`.
    pub tag: String,
    /// XBRL unit, e.g. `USD`, `shares`. A value without its unit is not a number.
    pub unit: String,
    /// Start of the period the fact describes; `None` for instantaneous facts
    /// (balance-sheet items are as-at a date, not over a span).
    pub period_start: Option<NaiveDate>,
    /// End of the period, or the instant, the fact describes.
    pub period_end: NaiveDate,
    pub fiscal_year: Option<i32>,
    /// `Q1`..`Q4`, `FY`.
    pub fiscal_period: Option<String>,
    /// `10-K`, `10-Q`, `8-K`, `10-K/A`, …
    pub form: String,
    /// Accession number of the filing that carried it — the provenance that makes
    /// the value re-derivable.
    pub accession: String,
    pub value: Decimal,
    /// **The knowledge time.** EDGAR's acceptance timestamp for the filing.
    pub accepted_at: DateTime<Utc>,
    /// True when `accepted_at` was derived from the filing *date* rather than read
    /// from the submission's `acceptanceDateTime` (ADR-P2-24).
    pub accepted_at_derived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FundamentalsError {
    /// A number accepted before the period it reports had finished. Impossible:
    /// it would mean the filer reported a quarter that had not happened.
    #[error(
        "cik {cik} tag {tag}: accepted_at {accepted_at} precedes period end {period_end} — \
         a filing cannot report a period that has not ended"
    )]
    AcceptedBeforePeriodEnd {
        cik: u32,
        tag: String,
        period_end: NaiveDate,
        accepted_at: DateTime<Utc>,
    },
    #[error("cik {cik} tag {tag}: period start {start} is after period end {end}")]
    InvertedPeriod {
        cik: u32,
        tag: String,
        start: NaiveDate,
        end: NaiveDate,
    },
    #[error("a fact must name its tag, unit and accession")]
    MissingProvenance,
}

impl FundamentalFact {
    /// # Errors
    /// Rejects a fact accepted before its period ended, an inverted period, and a
    /// fact with no provenance.
    pub fn validate(&self) -> Result<(), FundamentalsError> {
        if self.tag.trim().is_empty()
            || self.unit.trim().is_empty()
            || self.accession.trim().is_empty()
        {
            return Err(FundamentalsError::MissingProvenance);
        }
        if let Some(start) = self.period_start {
            if start > self.period_end {
                return Err(FundamentalsError::InvertedPeriod {
                    cik: self.cik,
                    tag: self.tag.clone(),
                    start,
                    end: self.period_end,
                });
            }
        }
        if self.accepted_at.date_naive() < self.period_end {
            return Err(FundamentalsError::AcceptedBeforePeriodEnd {
                cik: self.cik,
                tag: self.tag.clone(),
                period_end: self.period_end,
                accepted_at: self.accepted_at,
            });
        }
        Ok(())
    }
}

/// Every fact for `(cik, tag)` that was knowable at `as_of`, ordered by the period
/// they describe and then by acceptance.
///
/// This is the only filter. There is deliberately no variant that takes a period
/// bound without an `as_of`.
#[must_use]
pub fn facts_as_of<'a>(
    facts: &'a [FundamentalFact],
    cik: u32,
    tag: &str,
    as_of: DateTime<Utc>,
) -> Vec<&'a FundamentalFact> {
    let mut out: Vec<&FundamentalFact> = facts
        .iter()
        .filter(|f| f.cik == cik && f.tag == tag && f.accepted_at <= as_of)
        .collect();
    out.sort_by(|a, b| {
        a.period_end
            .cmp(&b.period_end)
            .then(a.accepted_at.cmp(&b.accepted_at))
    });
    out
}

/// The value a strategy would have used at `as_of`: the most recently *accepted*
/// fact for `(cik, tag)`.
///
/// Note the ordering — by acceptance, not by period. An amendment restating an
/// older quarter is newer information than an untouched later quarter's original
/// print, and a strategy reading the tape would have seen the amendment.
#[must_use]
pub fn latest_as_of<'a>(
    facts: &'a [FundamentalFact],
    cik: u32,
    tag: &str,
    as_of: DateTime<Utc>,
) -> Option<&'a FundamentalFact> {
    facts
        .iter()
        .filter(|f| f.cik == cik && f.tag == tag && f.accepted_at <= as_of)
        .max_by(|a, b| {
            a.accepted_at
                .cmp(&b.accepted_at)
                .then(a.period_end.cmp(&b.period_end))
        })
}

/// The fact describing `period_end` as it stood at `as_of` — the original print
/// until an amendment is accepted, the amendment afterwards.
#[must_use]
pub fn fact_for_period_as_of<'a>(
    facts: &'a [FundamentalFact],
    cik: u32,
    tag: &str,
    period_end: NaiveDate,
    as_of: DateTime<Utc>,
) -> Option<&'a FundamentalFact> {
    facts
        .iter()
        .filter(|f| {
            f.cik == cik && f.tag == tag && f.period_end == period_end && f.accepted_at <= as_of
        })
        .max_by_key(|f| f.accepted_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rust_decimal_macros::dec;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
    }

    fn t(y: i32, m: u32, day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, day, 12, 0, 0)
            .single()
            .expect("valid instant")
    }

    fn fact(period_end: NaiveDate, accepted: DateTime<Utc>, value: Decimal, form: &str) -> FundamentalFact {
        FundamentalFact {
            cik: 320_193,
            tag: "Revenues".into(),
            unit: "USD".into(),
            period_start: Some(period_end - chrono::Duration::days(89)),
            period_end,
            fiscal_year: Some(2023),
            fiscal_period: Some("Q1".into()),
            form: form.into(),
            accession: format!("0000320193-{}-000001", period_end.format("%y")),
            value,
            accepted_at: accepted,
            accepted_at_derived: false,
        }
    }

    /// DATA-005 §13 D9 (the criterion the plan cites as "§10.4"), adversarial half:
    /// the fact's period precedes its acceptance by a quarter, and a read inside
    /// that window must return NOTHING rather than the not-yet-filed number.
    #[test]
    fn a_fact_is_invisible_until_it_is_accepted() {
        let facts = vec![fact(d(2023, 3, 31), t(2023, 6, 30), dec!(94_836_000_000), "10-Q")];
        // The quarter has ended; the filing has not been accepted.
        assert!(latest_as_of(&facts, 320_193, "Revenues", t(2023, 5, 1)).is_none());
        assert!(facts_as_of(&facts, 320_193, "Revenues", t(2023, 5, 1)).is_empty());
        // After acceptance it appears.
        assert!(latest_as_of(&facts, 320_193, "Revenues", t(2023, 7, 1)).is_some());
    }

    /// An amendment is a new row and must not leak backwards.
    #[test]
    fn a_planted_amendment_does_not_leak_backwards() {
        let facts = vec![
            fact(d(2023, 3, 31), t(2023, 6, 30), dec!(100), "10-Q"),
            fact(d(2023, 3, 31), t(2023, 11, 1), dec!(90), "10-Q/A"),
        ];
        let before = fact_for_period_as_of(&facts, 320_193, "Revenues", d(2023, 3, 31), t(2023, 8, 1))
            .expect("the original print was knowable in August");
        assert_eq!(before.value, dec!(100));
        assert_eq!(before.form, "10-Q");

        let after = fact_for_period_as_of(&facts, 320_193, "Revenues", d(2023, 3, 31), t(2023, 12, 1))
            .expect("the amendment was knowable in December");
        assert_eq!(after.value, dec!(90));
        assert_eq!(after.form, "10-Q/A");
    }

    #[test]
    fn the_latest_accepted_fact_wins_over_the_later_period() {
        let facts = vec![
            fact(d(2023, 3, 31), t(2023, 5, 5), dec!(100), "10-Q"),
            fact(d(2023, 6, 30), t(2023, 8, 5), dec!(110), "10-Q"),
            // An amendment to Q1 accepted after Q2's original print.
            fact(d(2023, 3, 31), t(2023, 9, 5), dec!(95), "10-Q/A"),
        ];
        let latest = latest_as_of(&facts, 320_193, "Revenues", t(2023, 10, 1)).expect("some fact");
        assert_eq!(latest.value, dec!(95));
    }

    #[test]
    fn a_fact_accepted_before_its_period_ended_is_refused() {
        let bad = fact(d(2023, 3, 31), t(2023, 2, 1), dec!(1), "10-Q");
        assert!(matches!(
            bad.validate(),
            Err(FundamentalsError::AcceptedBeforePeriodEnd { .. })
        ));
    }

    #[test]
    fn a_fact_without_provenance_is_refused() {
        let mut bad = fact(d(2023, 3, 31), t(2023, 6, 30), dec!(1), "10-Q");
        bad.accession = String::new();
        assert_eq!(bad.validate(), Err(FundamentalsError::MissingProvenance));
    }

    #[test]
    fn a_well_formed_fact_validates() {
        assert_eq!(
            fact(d(2023, 3, 31), t(2023, 6, 30), dec!(1), "10-Q").validate(),
            Ok(())
        );
    }
}
