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
//!
//! The axis is **the receipt, not the transport.** A row the platform watched
//! arrive is observed whether it came over a WebSocket or an HTTP poll; a row
//! pulled from a history endpoint long afterwards is derived, because no receipt
//! for it was ever recorded. Keying this on push-versus-pull instead looks
//! conservative and is not: it marks every forward poll derived, so a forward
//! collector runs for months and still supplies Gate 15 with nothing, and it
//! overwrites a real receipt with a sentinel that can land *later* than the moment
//! the bytes were actually in hand.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::quality::QualityFlags;

/// How a source delivers data.
///
/// This records the transport and the shape of the endpoint. It does **not** decide
/// provenance — [`Collection`] does, per row, because one source commonly supplies
/// both forward polls and after-the-fact history.
///
/// Mirrors the `kind` column of `dataplane.source` (migration 0046).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A push stream.
    LiveStream,
    /// A pull endpoint. Rows the platform polls forward carry a real receipt and
    /// are observed; rows it pulls out of the endpoint's history do not and are
    /// derived.
    RestHistory,
    VendorFile,
    Derived,
}

impl SourceKind {
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
    /// Polled forward on a schedule, so its rows carry a real receipt and are
    /// **observed** — this is the leg that produces forward evidence for Gate 15
    /// (D-P3), and the reason switching this collector on is urgent.
    ///
    /// The declared lag of one funding hour is not this source's knowledge time; it
    /// is the bound a *backfill* through the same source would have to use, and what
    /// a dataset carries into `EmbargoInputs.max_knowledge_lag_ms`.
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
    /// Explicit history: a year of settlements nobody watched arrive. Every row is
    /// [`Collection::Historical`] by construction, and there is no call site that
    /// could say otherwise.
    #[must_use]
    pub const fn kraken_futures_funding_history() -> Self {
        Self {
            name: "kraken_futures_history",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(3600),
            notes: "historical-funding-rates; rates are published at the settlement hour",
        }
    }

    /// Tradier option chain snapshots.
    ///
    /// Delayed data: a forward snapshot IS observed — the platform held the bytes
    /// and knows when — but it observed a quote that was already 900 s stale. The
    /// receipt is the knowledge time; the 900 s is the declared lag the row carries
    /// into embargo arithmetic. Observed and observed-late are different facts and
    /// this source records both.
    #[must_use]
    pub const fn tradier_chain() -> Self {
        Self {
            name: "tradier_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(900),
            notes:
                "Tradier documents a 15-minute delay on sandbox data and on production index data",
        }
    }

    /// The knowledge time for one row, given how that row was collected.
    ///
    /// The axis that decides observed-versus-derived is **whether the platform
    /// holds a receipt for this particular row** — not whether the transport was a
    /// push or a pull. A WebSocket frame and an HTTP poll that the platform watched
    /// arrive are equally observed: in both cases there is a real instant at which
    /// the bytes were in hand, and recording it is the honest thing to do. A row
    /// pulled out of a history endpoint months later is derived, because no such
    /// instant was ever recorded and inventing one is the whole failure mode.
    ///
    /// Getting this axis wrong is not academic. Keying it on transport marks every
    /// forward poll as derived, which means a forward collector produces **no
    /// forward evidence at all** under D-P3 — the collector runs, the disk fills,
    /// and Gate 15 still has nothing. It also replaces a real receipt with
    /// `event_time + declared_lag`, which for a 05:00 settlement polled at 05:40
    /// stamps 06:00: an invented knowledge time *later* than the moment the
    /// platform actually held the bytes.
    ///
    /// Note what this function does not take: an override, an `Option`, or a
    /// defaulted flag. The caller must say which [`Collection`] this row came from,
    /// and there is no third answer.
    ///
    /// # Errors
    /// [`KnowledgeError::ObservedBeforeEvent`] if a receipt precedes the event it
    /// claims to observe. Nothing is observed before it happens, and a clock skew
    /// that says otherwise must surface rather than be written.
    pub fn knowledge_for(
        &self,
        event_time: DateTime<Utc>,
        how: Collection,
    ) -> Result<Knowledge, KnowledgeError> {
        match how {
            Collection::Forward { received_at } => {
                if received_at < event_time {
                    return Err(KnowledgeError::ObservedBeforeEvent {
                        received_at,
                        event_time,
                        source_name: self.name,
                    });
                }
                Ok(Knowledge::Observed { at: received_at })
            }
            Collection::Historical => {
                let lag = chrono::Duration::from_std(self.declared_lag)
                    .unwrap_or_else(|_| chrono::Duration::zero());
                Ok(Knowledge::Derived {
                    at: event_time + lag,
                    from_event_time: event_time,
                    declared_lag: self.declared_lag,
                })
            }
        }
    }
}

/// How one row reached the platform.
///
/// This is per-row, not per-source, because one source legitimately supplies both:
/// Kraken Futures' ticker is polled forward every hour *and* its
/// `historical-funding-rates` endpoint hands back a year nobody watched arrive.
/// The same `SourceDecl` produces observed rows for the first and derived rows for
/// the second, and the distinction has to travel with the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collection {
    /// The platform held the bytes at `received_at` and is recording that fact.
    /// The row is forward evidence (D-P3).
    Forward { received_at: DateTime<Utc> },
    /// Fetched from a history endpoint after the fact. No receipt exists, so the
    /// knowledge time is the declared sentinel and the row is flagged.
    Historical,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KnowledgeError {
    // `source_name`, not `source`: thiserror treats a field called `source` as the
    // error's cause and demands it implement `StdError`.
    #[error("source {source_name}: receipt {received_at} precedes the event {event_time} it claims to observe")]
    ObservedBeforeEvent {
        received_at: DateTime<Utc>,
        event_time: DateTime<Utc>,
        source_name: &'static str,
    },
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
    Derived {
        at: DateTime<Utc>,
        from_event_time: DateTime<Utc>,
        #[serde(with = "secs")]
        declared_lag: Duration,
    },
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

    fn fwd(at: DateTime<Utc>) -> Collection {
        Collection::Forward { received_at: at }
    }

    #[test]
    fn a_live_stream_observes_its_knowledge_time() {
        let live = SourceDecl {
            name: "kraken_ws",
            kind: SourceKind::LiveStream,
            declared_lag: Duration::ZERO,
            notes: "live",
        };
        let recv = t(1) + chrono::Duration::milliseconds(40);
        let k = live.knowledge_for(t(1), fwd(recv)).unwrap();
        assert!(!k.is_derived());
        assert!(k.is_forward_evidence());
        assert_eq!(k.quality_flags(), QualityFlags::NONE);
        assert_eq!(k.time(), recv);
    }

    /// The correction that matters most in this module: a **forward poll** is
    /// observed. Marking it derived because the transport was a pull would mean a
    /// collector running for months supplies Gate 15 with nothing (D-P3), which is
    /// the opposite of why P-0.3 is urgent.
    #[test]
    fn a_forward_rest_poll_is_observed_and_is_forward_evidence() {
        let src = SourceDecl::kraken_futures_ticker();
        let recv = t(5) + chrono::Duration::minutes(40);
        let k = src.knowledge_for(t(5), fwd(recv)).unwrap();
        assert!(
            !k.is_derived(),
            "a poll the platform watched arrive is observed, whatever the transport"
        );
        assert!(
            k.is_forward_evidence(),
            "this is the leg that feeds Gate 15"
        );
        assert_eq!(k.quality_flags(), QualityFlags::NONE);
        assert_eq!(
            k.time(),
            recv,
            "the real receipt is kept, not replaced by a sentinel"
        );
        let sentinel = src.knowledge_for(t(5), Collection::Historical).unwrap();
        assert!(
            sentinel.time() > recv,
            "the sentinel would have post-dated the real receipt"
        );
    }

    /// The original bug, as a test. A row pulled from a history endpoint has no
    /// receipt, so its knowledge time is the declared sentinel and it is flagged.
    #[test]
    fn a_historical_pull_is_derived_and_flagged() {
        let src = SourceDecl::kraken_futures_funding_history();
        let k = src.knowledge_for(t(1), Collection::Historical).unwrap();
        assert!(k.is_derived());
        assert!(
            !k.is_forward_evidence(),
            "the sentinel must never reach Gate 15's evidence (D-P3)"
        );
        assert!(k
            .quality_flags()
            .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(
            k.time(),
            t(2),
            "derived knowledge is event_time + the declared lag"
        );
    }

    /// Adversarial: nothing is observed before it happens. A receipt that precedes
    /// its event is clock skew or a parsing error, and it must surface rather than
    /// be written as a row that knows the future.
    #[test]
    fn a_receipt_before_its_event_is_refused() {
        let src = SourceDecl::kraken_futures_ticker();
        let err = src.knowledge_for(t(5), fwd(t(4))).unwrap_err();
        assert!(matches!(err, KnowledgeError::ObservedBeforeEvent { .. }));
        assert!(err.to_string().contains("precedes"));
        assert!(src.knowledge_for(t(5), fwd(t(5))).is_ok());
    }

    /// Adversarial: an observed row and a derived row for the same event must stay
    /// distinguishable through a serialization round trip. If they collapse to the
    /// same JSON, provenance is lost in transit and the discipline is decorative.
    #[test]
    fn observed_and_derived_stay_distinguishable_through_serde() {
        let src = SourceDecl::kraken_futures_ticker();
        let d = src.knowledge_for(t(1), Collection::Historical).unwrap();
        let o = src.knowledge_for(t(1), fwd(t(2))).unwrap();
        assert_ne!(d, o);
        let dj = serde_json::to_string(&d).unwrap();
        let oj = serde_json::to_string(&o).unwrap();
        assert_ne!(dj, oj);
        assert!(dj.contains("derived"), "{dj}");
        assert!(oj.contains("observed"), "{oj}");
        assert_eq!(serde_json::from_str::<Knowledge>(&dj).unwrap(), d);
        assert_eq!(serde_json::from_str::<Knowledge>(&oj).unwrap(), o);
        assert_eq!(o.time(), d.time());
        assert_ne!(o.is_derived(), d.is_derived());
        assert_ne!(o.quality_flags().0, d.quality_flags().0);
    }

    /// One source, both legs. This is why [`Collection`] is per row and not a
    /// property of the [`SourceDecl`].
    #[test]
    fn one_source_supplies_both_legs() {
        let src = SourceDecl::kraken_futures_ticker();
        let forward = src
            .knowledge_for(t(5), fwd(t(5) + chrono::Duration::minutes(1)))
            .unwrap();
        let history = src.knowledge_for(t(5), Collection::Historical).unwrap();
        assert!(forward.is_forward_evidence());
        assert!(!history.is_forward_evidence());
    }

    #[test]
    fn declared_lags_match_the_documented_source() {
        assert_eq!(
            SourceDecl::tradier_chain().declared_lag,
            Duration::from_secs(900)
        );
        assert_eq!(
            SourceDecl::kraken_futures_ticker().declared_lag,
            Duration::from_secs(3600)
        );
        for s in [
            SourceDecl::kraken_futures_ticker(),
            SourceDecl::kraken_futures_funding_history(),
            SourceDecl::tradier_chain(),
        ] {
            assert!(
                !s.notes.is_empty(),
                "{} declares a lag with no stated reason",
                s.name
            );
        }
    }

    #[test]
    fn implied_lag_is_what_a_dataset_must_carry() {
        let src = SourceDecl::tradier_chain();
        let k = src.knowledge_for(t(1), Collection::Historical).unwrap();
        assert_eq!(k.implied_lag(t(1)), Duration::from_secs(900));
    }
}
