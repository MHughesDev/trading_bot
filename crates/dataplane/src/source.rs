//! Source declarations and the knowledge-time discipline (Set P D-P2/D-P3;
//! ADR-P2-24; DATA-005 §11).
//!
//! Every row the platform stores must be able to answer one question: *did we
//! observe when we learned this, or did we infer it?* A row that cannot answer does
//! not get written. This module makes that answer a type, so that "cannot answer"
//! is a compile error rather than a default.
//!
//! This is not a hypothetical discipline. It shipped broken once: `market_bars_v2`
//! stamped REST backfills with the bar's close time, so a year of history that had
//! been fetched after the fact looked as though it had been watched live. Measured
//! on the live database on 2026-09-15, **all 158,259 rows** in that table carry
//! `available_time = event_time` — every single one, including the ones a REST
//! endpoint handed us weeks late. Gate 15 takes forward evidence from observed rows
//! only (D-P3), and with a stamp like that it cannot tell which those are.
//!
//! The rule in one line: **`knowledge_time` is never guessed.** It is either the
//! instant the platform received the bytes, or it is `event_time + the source's
//! declared lag` carrying [`QualityFlags::BACKFILLED_KNOWLEDGE_TIME`]. There is no
//! third option and no unflagged path between them.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::quality::QualityFlags;

/// How a source delivers data, which decides whether its knowledge time can be
/// observed at all.
///
/// Mirrors the `kind` column of `dataplane.source` (migration 0046).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A push stream. Receipt time *is* knowledge time, so it can be observed.
    LiveStream,
    /// A pull endpoint returning history. Whatever it returns was knowable before
    /// we asked, so our receipt time is not the knowledge time and must not be
    /// recorded as one.
    RestHistory,
    VendorFile,
    Derived,
}

impl SourceKind {
    /// Whether a row from this source may ever carry an observed knowledge time.
    ///
    /// `RestHistory` cannot, *even for a row that happens to be fresh*. A REST poll
    /// that catches a value seconds after it was published still did not observe
    /// its publication: it observed our poll. Treating a fresh pull as observed is
    /// how the sentinel leaks into Gate 15's evidence.
    #[must_use]
    pub const fn can_observe_knowledge(self) -> bool {
        matches!(self, Self::LiveStream)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LiveStream => "live_stream",
            Self::RestHistory => "rest_history",
            Self::VendorFile => "vendor_file",
            Self::Derived => "derived",
        }
    }
}

/// A source, its kind, and the lag it declares.
///
/// The declared lag is what a backfilled row's knowledge time is derived from, and
/// what a dataset carries into `EmbargoInputs.max_knowledge_lag_ms` so the sentinel
/// cannot flatter a result (Set P §6 exit 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecl {
    /// Must match a `dataplane.source.name`.
    pub name: &'static str,
    pub kind: SourceKind,
    pub declared_lag: Duration,
    /// Why the lag is what it is, in one sentence.
    pub notes: &'static str,
}

impl SourceDecl {
    /// Kraken Futures public `/derivatives` ticker poll: funding rate, funding rate
    /// prediction and open interest.
    ///
    /// REST, therefore never observed. The declared lag is one funding hour, the
    /// interval at which the underlying value changes: a value read from the ticker
    /// could have been published at any point since the last hourly settlement, and
    /// the honest bound is the whole interval, not the round-trip time.
    #[must_use]
    pub const fn kraken_futures_ticker() -> Self {
        Self {
            name: "kraken_futures_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(3600),
            notes: "public /derivatives/api/v3/tickers poll; funding settles hourly, so a polled value is bounded by one settlement interval",
        }
    }

    /// Kraken Futures `/derivatives/api/v3/historical-funding-rates`.
    ///
    /// Explicit history. Every row is a backfill by construction.
    #[must_use]
    pub const fn kraken_futures_funding_history() -> Self {
        Self {
            name: "kraken_futures_history",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(3600),
            notes: "historical-funding-rates; rates are published at the settlement hour",
        }
    }

    /// Tradier option chain snapshots. Delayed data: the row is observed, but
    /// observed *late*, and those are not the same thing — so it is a REST source
    /// with a 900 s declared lag, not a live one.
    #[must_use]
    pub const fn tradier_chain() -> Self {
        Self {
            name: "tradier_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(900),
            notes: "Tradier documents a 15-minute delay on sandbox data and on production index data",
        }
    }

    /// Derive the knowledge time for a row from this source.
    ///
    /// `received_at` is when the platform had the bytes; `event_time` is when the
    /// fact happened. A live stream's knowledge time is the receipt. Anything else
    /// is `event_time + declared_lag`, flagged.
    ///
    /// Note what this function does *not* take: an override, an `Option`, or a
    /// "trust me, it's live" flag. The source's kind decides, and a caller cannot
    /// argue with it.
    #[must_use]
    pub fn knowledge_for(&self, event_time: DateTime<Utc>, received_at: DateTime<Utc>) -> Knowledge {
        if self.kind.can_observe_knowledge() {
            Knowledge::Observed { at: received_at }
        } else {
            let lag = chrono::Duration::from_std(self.declared_lag).unwrap_or_else(|_| chrono::Duration::zero());
            Knowledge::Derived { at: event_time + lag, from_event_time: event_time, declared_lag: self.declared_lag }
        }
    }
}

/// When the platform could first have known a fact — and, inseparably, whether that
/// instant was observed or inferred.
///
/// There is deliberately no `Default`, no `Option<bool>` beside it and no
/// constructor that takes a raw timestamp without saying which kind it is. A row
/// that cannot state its provenance cannot be built, and so cannot be written
/// (Set P §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provenance", rename_all = "snake_case")]
pub enum Knowledge {
    /// The platform watched this arrive. `at` is receipt.
    Observed { at: DateTime<Utc> },
    /// The platform inferred when it could have known. Carries the sentinel.
    Derived { at: DateTime<Utc>, from_event_time: DateTime<Utc>, #[serde(with = "secs")] declared_lag: Duration },
}

impl Knowledge {
    #[must_use]
    pub const fn time(&self) -> DateTime<Utc> {
        match self {
            Self::Observed { at } | Self::Derived { at, .. } => *at,
        }
    }

    #[must_use]
    pub const fn is_derived(&self) -> bool {
        matches!(self, Self::Derived { .. })
    }

    /// Whether Gate 15 may take forward evidence from a row with this stamp (D-P3).
    #[must_use]
    pub const fn is_forward_evidence(&self) -> bool {
        matches!(self, Self::Observed { .. })
    }

    /// The flags a row with this stamp must carry. A derived row that reached
    /// storage without this flag is the bug this module exists to prevent.
    #[must_use]
    pub const fn quality_flags(&self) -> QualityFlags {
        match self {
            Self::Observed { .. } => QualityFlags::NONE,
            Self::Derived { .. } => QualityFlags::BACKFILLED_KNOWLEDGE_TIME,
        }
    }

    /// The lag this stamp implies, for `EmbargoInputs.max_knowledge_lag_ms`.
    #[must_use]
    pub fn implied_lag(&self, event_time: DateTime<Utc>) -> Duration {
        let d = self.time() - event_time;
        d.to_std().unwrap_or(Duration::ZERO)
    }
}

/// `Duration` as whole seconds, so the JSON form is readable and stable.
mod secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(d)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, h, 0, 0).unwrap()
    }

    #[test]
    fn a_live_stream_observes_its_knowledge_time() {
        let live = SourceDecl {
            name: "kraken_ws",
            kind: SourceKind::LiveStream,
            declared_lag: Duration::ZERO,
            notes: "live",
        };
        let k = live.knowledge_for(t(1), t(1) + chrono::Duration::milliseconds(40));
        assert!(!k.is_derived());
        assert!(k.is_forward_evidence());
        assert_eq!(k.quality_flags(), QualityFlags::NONE);
        assert_eq!(k.time(), t(1) + chrono::Duration::milliseconds(40));
    }

    /// The bug, as a test. A REST poll that happens to catch a fresh value must
    /// still be derived: we observed our poll, not the publication.
    #[test]
    fn a_fresh_rest_poll_is_still_derived_not_observed() {
        let rest = SourceDecl::kraken_futures_ticker();
        // Received one second after the event — as fresh as a poll can be.
        let k = rest.knowledge_for(t(1), t(1) + chrono::Duration::seconds(1));
        assert!(k.is_derived(), "a REST read is never observed knowledge, however fresh");
        assert!(!k.is_forward_evidence(), "the sentinel must never reach Gate 15's evidence (D-P3)");
        assert!(k.quality_flags().contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(k.time(), t(2), "derived knowledge is event_time + the declared lag, not receipt");
    }

    /// Adversarial: an observed row and a derived row for the same event must be
    /// distinguishable after a serialization round trip. If they collapse to the
    /// same JSON, provenance is lost in transit and the whole discipline is
    /// decorative.
    #[test]
    fn observed_and_derived_stay_distinguishable_through_serde() {
        let rest = SourceDecl::kraken_futures_ticker();
        let live = SourceDecl { name: "x", kind: SourceKind::LiveStream, declared_lag: Duration::ZERO, notes: "" };
        let d = rest.knowledge_for(t(1), t(1));
        let o = live.knowledge_for(t(1), t(2));
        assert_ne!(d, o);
        let dj = serde_json::to_string(&d).unwrap();
        let oj = serde_json::to_string(&o).unwrap();
        assert_ne!(dj, oj);
        assert!(dj.contains("derived"), "{dj}");
        assert!(oj.contains("observed"), "{oj}");
        assert_eq!(serde_json::from_str::<Knowledge>(&dj).unwrap(), d);
        assert_eq!(serde_json::from_str::<Knowledge>(&oj).unwrap(), o);
        // Even when both land on the same instant, the provenance differs.
        let same_instant = live.knowledge_for(t(1), t(2));
        assert_eq!(same_instant.time(), d.time());
        assert_ne!(same_instant.is_derived(), d.is_derived());
    }

    #[test]
    fn only_a_live_stream_may_observe() {
        assert!(SourceKind::LiveStream.can_observe_knowledge());
        for k in [SourceKind::RestHistory, SourceKind::VendorFile, SourceKind::Derived] {
            assert!(!k.can_observe_knowledge(), "{} must not claim observed knowledge", k.as_str());
        }
    }

    #[test]
    fn declared_lags_match_the_documented_source() {
        assert_eq!(SourceDecl::tradier_chain().declared_lag, Duration::from_secs(900));
        assert_eq!(SourceDecl::kraken_futures_ticker().declared_lag, Duration::from_secs(3600));
        for s in [
            SourceDecl::kraken_futures_ticker(),
            SourceDecl::kraken_futures_funding_history(),
            SourceDecl::tradier_chain(),
        ] {
            assert!(!s.notes.is_empty(), "{} declares a lag with no stated reason", s.name);
        }
    }

    #[test]
    fn implied_lag_is_what_a_dataset_must_carry() {
        let rest = SourceDecl::tradier_chain();
        let k = rest.knowledge_for(t(1), t(1));
        assert_eq!(k.implied_lag(t(1)), Duration::from_secs(900));
    }
}
