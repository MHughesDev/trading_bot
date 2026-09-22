//! Backfilled and observed rows, kept distinguishable *structurally* rather
//! than by memory (Set P P-1.6; ADR-P2-24; INV-01; D-P2, D-P3).
//!
//! ## The failure this exists to make impossible
//!
//! It has already happened once. REST backfills were stamped with their bar
//! close time, so months of history claimed to have been knowable the instant
//! the candle ended. Nothing in the platform could tell a bar it had *watched*
//! arrive from one handed to it weeks later, and every study over that window
//! silently assumed the former. The canonical migration flagged the rows; this
//! module is the part that makes the property hold for rows nobody has written
//! yet.
//!
//! ## The axis is the receipt, per row — never the transport
//!
//! A source is not "live" or "historical"; a *row* is observed or backfilled:
//!
//! * A **forward poll** of a REST candle endpoint thirty seconds after the
//!   close observed that bar. Marking it backfilled because it arrived over
//!   HTTP buries a real receipt under a sentinel.
//! * A **bulk backfill** from a websocket-capable venue did not observe
//!   anything; its receipt is today, which says nothing about when the bar
//!   could have been known.
//! * A **delayed source polled forward** can receive a bar *later* than its
//!   declared lag. Stamping `close + lag` there claims the platform knew it
//!   before it did — lookahead, the one direction that flatters a result.
//!
//! So the decision is: if the row arrived within its source's declared lag plus
//! an operational margin, the receipt is meaningful and `knowledge_time` is the
//! receipt. Otherwise the receipt is uninformative, the declared sentinel is
//! used, and the row wears [`QualityFlags::BACKFILLED_KNOWLEDGE_TIME`] forever.
//!
//! ## Flag, never block — but absorb it structurally
//!
//! Refusing backfilled rows would remove essentially all the history the
//! platform has, to guard against a bounded effect. Instead the sentinel is
//! absorbed: a dataset's sources' **maximum declared lag** enters
//! `EmbargoInputs::max_knowledge_lag_ms`, so the computed embargo already
//! exceeds any error the sentinel could introduce. The one place the boundary
//! is a hard line is money — Gate 15's forward-test evidence must come from
//! observed rows only ([`KnowledgeMix::admissible_as_forward_evidence`]).

use serde::{Deserialize, Serialize};

use crate::quality::QualityFlags;

/// One source's contribution to a window, as the dataset builder counted it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRows {
    /// `dataplane.source.name`.
    pub source: String,
    /// `dataplane.source.declared_vendor_lag_ms` — read from the table, never
    /// assumed. Coinbase and Kraken declare 60 000; Alpaca declares 900 000,
    /// and treating them as equal is a fourteen-minute lookahead on every
    /// Alpaca bar.
    pub declared_lag_ms: u64,
    pub rows: u64,
    /// Of `rows`, how many carry `BACKFILLED_KNOWLEDGE_TIME`.
    pub backfilled_rows: u64,
}

/// What a built dataset knows about its own knowledge times.
///
/// Recorded in the dataset manifest rather than computed at read time: the
/// fraction of a dataset that is backfilled is a property of the rows that
/// existed when it was built, and a later backfill must not retroactively
/// change what a finished study claimed about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeMix {
    pub sources: Vec<SourceRows>,
}

impl KnowledgeMix {
    #[must_use]
    pub fn new(sources: Vec<SourceRows>) -> Self {
        let mut s = sources;
        // Sorted so the manifest hash does not depend on the order the builder
        // happened to visit its sources in.
        s.sort_by(|a, b| a.source.cmp(&b.source));
        Self { sources: s }
    }

    #[must_use]
    pub fn total_rows(&self) -> u64 {
        self.sources.iter().map(|s| s.rows).sum()
    }

    #[must_use]
    pub fn backfilled_rows(&self) -> u64 {
        self.sources.iter().map(|s| s.backfilled_rows).sum()
    }

    /// Share of the dataset whose knowledge time was derived, not observed.
    ///
    /// Stated in the manifest so a reader can see it. **Never a refusal**: a
    /// dataset that is entirely backfilled is flagged, not rejected (D-P2).
    #[must_use]
    pub fn backfilled_fraction(&self) -> f64 {
        let total = self.total_rows();
        if total == 0 {
            return 0.0;
        }
        // Ratio of counts, not of money — `f64` is the right type here.
        self.backfilled_rows() as f64 / total as f64
    }

    /// The number that must reach `EmbargoInputs::max_knowledge_lag_ms`.
    ///
    /// The **maximum** across contributing sources, not the average and not the
    /// first: a dataset that mixes Coinbase's 60 s with Alpaca's 900 s is only
    /// as certain as its slowest source, and taking anything less would let the
    /// mixture flatter the result exactly where the sentinel is largest.
    #[must_use]
    pub fn max_declared_lag_ms(&self) -> u64 {
        self.sources.iter().filter(|s| s.rows > 0).map(|s| s.declared_lag_ms).max().unwrap_or(0)
    }

    /// Whether this dataset may stand as Gate 15 forward-test evidence.
    ///
    /// **The one hard line** (D-P3). Everywhere else the sentinel is flagged and
    /// absorbed; here it is refused, because forward-test evidence is the claim
    /// that the platform saw a result arrive in real time, and a derived
    /// knowledge time is precisely the absence of that claim.
    #[must_use]
    pub fn admissible_as_forward_evidence(&self) -> bool {
        self.total_rows() > 0 && self.backfilled_rows() == 0
    }
}

/// Is a single row's knowledge time observed or derived?
///
/// One function, so no caller re-derives the test from the flag's name.
#[must_use]
pub const fn is_backfilled(flags: QualityFlags) -> bool {
    flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
}

/// Are these two rows for the same bar distinguishable as observed vs derived?
///
/// The P-1.6 property, written once so the test and any future caller agree on
/// what "distinguishable" means: the flags differ **and** the knowledge times
/// differ. Either alone is not enough — a flag with equal knowledge times
/// distinguishes them for a reader who looks, and different knowledge times
/// with equal flags distinguishes them for a query, and only both together
/// survive somebody dropping one of the two.
#[must_use]
pub fn distinguishable(a: (QualityFlags, i64), b: (QualityFlags, i64)) -> bool {
    is_backfilled(a.0) != is_backfilled(b.0) && a.1 != b.1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mix() -> KnowledgeMix {
        KnowledgeMix::new(vec![
            SourceRows { source: "coinbase_rest".into(), declared_lag_ms: 60_000, rows: 900, backfilled_rows: 900 },
            SourceRows { source: "alpaca_rest".into(), declared_lag_ms: 900_000, rows: 100, backfilled_rows: 50 },
        ])
    }

    /// The core of P-1.6: a mixed window carries the **maximum** declared lag of
    /// its sources, not the first and not the average.
    #[test]
    fn a_mixed_window_carries_the_largest_declared_lag_of_its_sources() {
        assert_eq!(mix().max_declared_lag_ms(), 900_000);
        // The adversarial half: a source that contributed no rows cannot inflate
        // the embargo, and one that contributed rows cannot be averaged away.
        let with_idle = KnowledgeMix::new(vec![
            SourceRows { source: "coinbase_rest".into(), declared_lag_ms: 60_000, rows: 1_000, backfilled_rows: 0 },
            SourceRows { source: "tradier_rest".into(), declared_lag_ms: 900_000, rows: 0, backfilled_rows: 0 },
        ]);
        assert_eq!(with_idle.max_declared_lag_ms(), 60_000);
    }

    #[test]
    fn a_fully_backfilled_dataset_is_flagged_not_refused() {
        let all = KnowledgeMix::new(vec![SourceRows {
            source: "coinbase_rest".into(),
            declared_lag_ms: 60_000,
            rows: 500,
            backfilled_rows: 500,
        }]);
        assert!((all.backfilled_fraction() - 1.0).abs() < f64::EPSILON);
        // Flagged, usable, and stated — never blocked (D-P2).
        assert!(all.total_rows() > 0);
    }

    /// D-P3: the sentinel never reaches money. Forward-test evidence must be
    /// observed rows only, and one backfilled row is enough to disqualify it.
    #[test]
    fn one_backfilled_row_disqualifies_a_window_as_forward_evidence() {
        let observed = KnowledgeMix::new(vec![SourceRows {
            source: "kraken_ws".into(),
            declared_lag_ms: 0,
            rows: 1_000,
            backfilled_rows: 0,
        }]);
        assert!(observed.admissible_as_forward_evidence());

        let mut tainted = observed.clone();
        tainted.sources[0].backfilled_rows = 1;
        assert!(!tainted.admissible_as_forward_evidence(), "one derived row is not forward evidence");
        assert!(!mix().admissible_as_forward_evidence());
        assert!(!KnowledgeMix::default().admissible_as_forward_evidence(), "an empty window proves nothing");
    }

    #[test]
    fn the_mix_is_order_independent() {
        let mut reversed = mix().sources;
        reversed.reverse();
        assert_eq!(KnowledgeMix::new(reversed), mix());
    }

    /// Distinguishability needs BOTH signals. This is the test that fails if
    /// somebody drops the flag *or* collapses the knowledge times.
    #[test]
    fn distinguishability_requires_both_the_flag_and_the_knowledge_time() {
        let observed = (QualityFlags::NONE, 1_000);
        let backfilled = (QualityFlags::BACKFILLED_KNOWLEDGE_TIME, 1_060);
        assert!(distinguishable(observed, backfilled));

        // The flag dropped: the rows still differ in time, but nothing says why.
        assert!(!distinguishable(observed, (QualityFlags::NONE, 1_060)));
        // The knowledge times collapsed: a query cannot separate them.
        assert!(!distinguishable(observed, (QualityFlags::BACKFILLED_KNOWLEDGE_TIME, 1_000)));
    }

    #[test]
    fn an_empty_mix_has_no_lag_and_no_backfilled_share() {
        let empty = KnowledgeMix::default();
        assert_eq!(empty.max_declared_lag_ms(), 0);
        assert!(empty.backfilled_fraction().abs() < f64::EPSILON);
    }
}
