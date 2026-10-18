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

/// What a source costs, and therefore whether it may be here at all
/// (Set P P-4.5; DA-14; ADR-0027).
///
/// ADR-0027 makes the platform free-only. Until now that was a convention: a paid
/// source could enter through a dependency, a trial key or a helpful pull request,
/// and nothing would notice until an invoice arrived or a backtest stopped being
/// reproducible on a machine without the key. This enum makes it a declaration.
///
/// There is deliberately **no `Default`**. A source cannot be constructed without
/// stating what it costs, so "nobody thought about it" is a compile error rather
/// than a silent `free`. The point is not to forbid paid data — it is to make
/// adding it a decision someone signs (a row in
/// [`PAID_SOURCE_ALLOWLIST`] naming the ADR) rather than a dependency that
/// appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    /// No payment, no key that costs money, no trial that expires into one.
    Free,
    /// Costs money. Admissible only with an allowlist entry naming the ADR that
    /// approved it.
    Paid,
}

impl CostClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Paid => "paid",
        }
    }

    /// Parse the stored form (`dataplane.source.cost_class`).
    ///
    /// Returns `None` for anything else, **including an empty string**: a column
    /// the migration left unset must fail the fence rather than be read as free.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "free" => Some(Self::Free),
            "paid" => Some(Self::Paid),
            _ => None,
        }
    }
}

/// One signed exception to the free-only rule.
///
/// `adr` is the document that approved the spend. It is required and checked
/// non-empty by [`check_cost_class`], because an allowlist entry with no ADR is
/// the convention this mechanism replaces, written down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaidSourceWaiver {
    /// `dataplane.source.name`.
    pub source: &'static str,
    /// e.g. `"ADR-0041"`. Never empty.
    pub adr: &'static str,
    /// What it costs and why it was worth it, in one sentence.
    pub note: &'static str,
}

/// Every paid source the platform has ever approved.
///
/// **Empty, and that is the current state of the programme**: ADR-0027 is
/// free-only and no ADR has reversed it. An addition here is a deliberate,
/// reviewable act — which is the entire mechanism.
pub const PAID_SOURCE_ALLOWLIST: &[PaidSourceWaiver] = &[];

/// Why a source may not enter the platform.
///
/// Every variant names its field `source_name`, not `source`: `thiserror` gives a
/// field called `source` error-chain meaning and demands it implement `StdError`.
/// The same trap `KnowledgeError` documents, two hundred lines down.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FenceError {
    #[error("source {source_name} declares cost_class = 'paid' but no entry in PAID_SOURCE_ALLOWLIST names the ADR that approved it; ADR-0027 makes this platform free-only, and adding a paid source is a decision to be signed rather than a dependency that appears (DA-14)")]
    PaidWithoutAdr { source_name: String },
    #[error("source {source_name} declares cost_class {value:?}, which is neither 'free' nor 'paid'; an unrecognised or absent cost class is refused rather than assumed free")]
    UnknownCostClass { source_name: String, value: String },
    #[error("PAID_SOURCE_ALLOWLIST entry for {source_name} names no ADR; an allowlist with an unsigned entry is the convention this check replaces")]
    WaiverNamesNoAdr { source_name: String },
}

/// Check one source against the shipped allowlist.
///
/// # Errors
/// [`FenceError`] when a paid source has no ADR-bearing waiver.
pub fn check_cost_class(name: &str, cost: CostClass) -> Result<(), FenceError> {
    check_cost_class_against(name, cost, PAID_SOURCE_ALLOWLIST)
}

/// The same check against a given allowlist.
///
/// Parameterised for one reason: the shipped allowlist is empty, so a test that
/// could only ever exercise the refusal would prove the fence is *closed* rather
/// than that it has a gate. Both branches are the same function.
///
/// # Errors
/// [`FenceError`] when a paid source has no ADR-bearing waiver in `allowlist`.
pub fn check_cost_class_against(
    name: &str,
    cost: CostClass,
    allowlist: &[PaidSourceWaiver],
) -> Result<(), FenceError> {
    if cost == CostClass::Free {
        return Ok(());
    }
    let waiver =
        allowlist
            .iter()
            .find(|w| w.source == name)
            .ok_or_else(|| FenceError::PaidWithoutAdr {
                source_name: name.to_string(),
            })?;
    if waiver.adr.trim().is_empty() {
        return Err(FenceError::WaiverNamesNoAdr {
            source_name: name.to_string(),
        });
    }
    Ok(())
}

/// Check sources as a build guard reads them out of the migrations: a name and
/// whatever text the `cost_class` column holds.
///
/// This is the entry point an `xtask` guard calls, so the rule lives beside the
/// declarations rather than being restated in a grep. `None` is what an absent
/// column looks like, and it fails.
///
/// # Errors
/// The first [`FenceError`] found.
pub fn check_declared_cost_classes<'a>(
    declared: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> Result<(), FenceError> {
    for (name, value) in declared {
        let raw = value.unwrap_or("");
        let class = CostClass::parse(raw).ok_or_else(|| FenceError::UnknownCostClass {
            source_name: name.to_string(),
            value: raw.to_string(),
        })?;
        check_cost_class(name, class)?;
    }
    Ok(())
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
    /// What this source costs. No default: see [`CostClass`].
    pub cost_class: CostClass,
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
            cost_class: CostClass::Free,
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
            cost_class: CostClass::Free,
            notes: "historical-funding-rates; rates are published at the settlement hour",
        }
    }

    /// Tradier option chain snapshots (P-0.5).
    ///
    /// Delayed data: a forward snapshot IS observed — the platform held the bytes
    /// and knows when — but it observed a quote that was already 900 s stale. The
    /// receipt is the knowledge time; the 900 s is the declared lag the row carries
    /// into embargo arithmetic. Observed and observed-late are different facts and
    /// this source records both.
    ///
    /// The name is `tradier_chain_rest`, the row migration 0098 created for chain
    /// snapshots — **not** `tradier_rest`, which migration 0046 seeded for the
    /// delayed *quote* collector. Both rows happen to declare 900 s, so the lag
    /// would have come out right either way; the id would not, and a row stamped
    /// with the wrong source names the wrong endpoint for every later provenance
    /// question.
    #[must_use]
    pub const fn tradier_chain() -> Self {
        Self {
            name: "tradier_chain_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(900),
            cost_class: CostClass::Free,
            notes:
                "Tradier documents a 15-minute delay on sandbox data and on production index data",
        }
    }

    /// Coinbase Exchange public candles (P-1.2).
    ///
    /// Mirrors `dataplane.source` row 2 (migration 0046). Declared in code as well
    /// as in the table because the backfill adapters name a source by string and
    /// nothing in the workspace could previously answer "what lag does that name
    /// declare?" without a database — which made P-1.4's cross-source property
    /// untestable.
    #[must_use]
    pub const fn coinbase_rest() -> Self {
        Self {
            name: "coinbase_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(60),
            cost_class: CostClass::Free,
            notes: "candles published after the minute closes",
        }
    }

    /// Kraken public OHLC and downloadable history archives (P-1.3).
    ///
    /// Mirrors `dataplane.source` row 3.
    #[must_use]
    pub const fn kraken_rest() -> Self {
        Self {
            name: "kraken_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(60),
            cost_class: CostClass::Free,
            notes: "public OHLC and history archives; candles published after the minute closes",
        }
    }

    /// Kraken's downloadable OHLCVT history archives (P-1.3).
    ///
    /// Its own source, not `kraken_rest`: the backfill router keys on
    /// `source_name()`, so two adapters answering one name cannot both be routed,
    /// and a vendor file is a different provenance from a REST page even when the
    /// candles agree. Same 60 s sentinel as the API path, because it is the same
    /// venue's same candles; mirrors `dataplane.source` row 23 (migration 0156).
    #[must_use]
    pub const fn kraken_archive() -> Self {
        Self {
            name: "kraken_archive",
            kind: SourceKind::VendorFile,
            declared_lag: Duration::from_secs(60),
            cost_class: CostClass::Free,
            notes: "Kraken OHLCVT history archives, verified by sha256 before any row is read",
        }
    }

    /// Alpaca free-plan historical bars (P-1.4).
    ///
    /// Mirrors `dataplane.source` row 5, and the 900 s is the point: it is fifteen
    /// times Coinbase's, so two backfills of the *same bar close* from the two
    /// sources land fifteen minutes apart in knowledge time. A dataset that spans
    /// both must carry the larger (ADR-P2-24), and assuming the lags equal is how a
    /// mixed-venue history quietly claims to have been knowable sooner than it was.
    #[must_use]
    pub const fn alpaca_rest() -> Self {
        Self {
            name: "alpaca_rest",
            kind: SourceKind::RestHistory,
            declared_lag: Duration::from_secs(900),
            cost_class: CostClass::Free,
            notes: "free-tier SIP data is 15-minute delayed",
        }
    }

    /// Every source declared in code, so a test can sweep them.
    pub const ALL: &'static [fn() -> Self] = &[
        Self::kraken_futures_ticker,
        Self::kraken_futures_funding_history,
        Self::tradier_chain,
        Self::coinbase_rest,
        Self::kraken_rest,
        Self::kraken_archive,
        Self::alpaca_rest,
    ];

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

/// The declared lag a dataset over a mixed window must carry (Set P P-1.6;
/// ADR-P2-24).
///
/// A dataset assembled from more than one source is only as timely as its *slowest*
/// source. Taking the mean, the first, or the source that happens to supply most of
/// the rows all understate the lag, and understating it shortens the embargo — which
/// is the direction that flatters a result. So the answer is the maximum, and it is
/// computed rather than typed.
///
/// Returns `None` for an empty mix: a dataset that names no source has no declared
/// lag to carry, and defaulting that to zero would be the worst possible guess.
#[must_use]
pub fn max_declared_lag<'a>(sources: impl IntoIterator<Item = &'a SourceDecl>) -> Option<Duration> {
    sources.into_iter().map(|s| s.declared_lag).max()
}

/// The same figure in the unit `split::EmbargoInputs.max_knowledge_lag_ms` wants.
///
/// # Errors
/// [`LagError::NoSources`] when the mix is empty, so a caller cannot silently reach
/// `max_knowledge_lag_ms = 0` for a dataset whose sources were never resolved.
pub fn max_knowledge_lag_ms<'a>(
    sources: impl IntoIterator<Item = &'a SourceDecl>,
) -> Result<u64, LagError> {
    max_declared_lag(sources)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .ok_or(LagError::NoSources)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LagError {
    #[error("a dataset must name at least one source before it can declare a knowledge lag; refusing to default to zero")]
    NoSources,
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
            cost_class: CostClass::Free,
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

    /// Every source declared in code names the row the migrations seeded, with the
    /// lag that row carries. A name that drifts resolves to the wrong `source_id`,
    /// and a row stamped with the wrong source carries the wrong declared lag into
    /// every embargo calculation that reads it.
    #[test]
    fn every_declared_source_matches_the_row_the_migrations_seeded() {
        // (name, declared_lag_ms) exactly as `migrations/0046_dataplane.sql` and
        // `migrations/0098_forward_series.sql` INSERT them.
        let seeded: &[(&str, u64)] = &[
            ("coinbase_rest", 60_000),
            ("kraken_rest", 60_000),
            ("kraken_archive", 60_000),
            ("alpaca_rest", 900_000),
            ("kraken_futures_rest", 3_600_000),
            ("kraken_futures_history", 3_600_000),
            ("tradier_chain_rest", 900_000),
        ];
        for make in SourceDecl::ALL {
            let s = make();
            let (_, ms) = seeded
                .iter()
                .find(|(n, _)| *n == s.name)
                .unwrap_or_else(|| panic!("{} is declared in code but seeded nowhere", s.name));
            assert_eq!(
                s.declared_lag,
                Duration::from_millis(*ms),
                "{} declares a lag the migration does not",
                s.name
            );
            assert!(
                !s.notes.is_empty(),
                "{} declares a lag with no reason",
                s.name
            );
        }
        assert_eq!(SourceDecl::ALL.len(), seeded.len());
    }

    // ── the free-only fence (P-4.5, DA-14, ADR-0027) ─────────────────────────

    /// The state of the programme, asserted rather than assumed: every source the
    /// platform declares is free, and the paid allowlist is empty because no ADR
    /// has reversed ADR-0027.
    #[test]
    fn every_declared_source_is_free_and_the_paid_allowlist_is_empty() {
        for make in SourceDecl::ALL {
            let s = make();
            assert_eq!(
                s.cost_class,
                CostClass::Free,
                "{} is not free; if that is intentional it needs an ADR and an allowlist entry",
                s.name
            );
            assert!(check_cost_class(s.name, s.cost_class).is_ok());
        }
        assert!(
            PAID_SOURCE_ALLOWLIST.is_empty(),
            "ADR-0027 is free-only; an entry here means an ADR reversed it, and this assertion is \
             where that shows up"
        );
    }

    /// P-4.5 clause 1. A paid source with nobody's signature on it is refused, and
    /// the refusal says what would admit it.
    #[test]
    fn a_paid_source_with_no_allowlist_entry_is_refused() {
        let err = check_cost_class("polygon_rest", CostClass::Paid).unwrap_err();
        assert_eq!(
            err,
            FenceError::PaidWithoutAdr {
                source_name: "polygon_rest".into()
            }
        );
        let text = err.to_string();
        assert!(text.contains("ADR-0027"), "{text}");
        assert!(
            text.contains("decision to be signed"),
            "the refusal should say what would admit it: {text}"
        );
    }

    /// P-4.5 clause 2. The mechanism is not a prohibition: an entry naming the ADR
    /// that approved the spend passes. Exercised through the real predicate over a
    /// stand-in allowlist, because the shipped one is empty and a test that only
    /// ever sees the refusal proves the fence is closed rather than that it has a
    /// gate.
    #[test]
    fn a_paid_source_named_in_an_allowlist_entry_with_its_adr_passes() {
        let signed = [PaidSourceWaiver {
            source: "polygon_rest",
            adr: "ADR-0041",
            note: "$29/month starter plan, approved for the equities onboarding",
        }];
        assert!(check_cost_class_against("polygon_rest", CostClass::Paid, &signed).is_ok());
        assert_eq!(
            check_cost_class_against("some_other_vendor", CostClass::Paid, &signed).unwrap_err(),
            FenceError::PaidWithoutAdr {
                source_name: "some_other_vendor".into()
            },
            "a waiver admits the source it names and no other"
        );

        // An entry with no ADR is itself refused: an unsigned allowlist is the
        // convention this check replaces, written down.
        let unsigned = [PaidSourceWaiver {
            source: "polygon_rest",
            adr: "   ",
            note: "nobody signed this",
        }];
        let err = check_cost_class_against("polygon_rest", CostClass::Paid, &unsigned).unwrap_err();
        assert_eq!(
            err,
            FenceError::WaiverNamesNoAdr {
                source_name: "polygon_rest".into()
            }
        );
        assert!(err.to_string().contains("unsigned entry"), "{err}");

        // A free source needs no waiver in any allowlist.
        assert!(check_cost_class_against("coinbase_rest", CostClass::Free, &[]).is_ok());
    }

    /// P-4.5 clause 3, the half a migration CHECK cannot cover on its own: a
    /// `cost_class` that is absent, empty or unrecognised **fails** rather than
    /// being read as free. An absent column is the case that would otherwise
    /// default to the answer nobody chose.
    #[test]
    fn a_source_with_no_cost_class_is_refused_rather_than_assumed_free() {
        assert_eq!(CostClass::parse("free"), Some(CostClass::Free));
        assert_eq!(CostClass::parse("paid"), Some(CostClass::Paid));
        for absent in ["", " ", "FREE", "gratis", "unknown"] {
            assert_eq!(
                CostClass::parse(absent),
                None,
                "{absent:?} must not parse as a cost class"
            );
        }
        // What a build guard reading the migrations sees.
        assert!(check_declared_cost_classes([("coinbase_rest", Some("free"))]).is_ok());
        assert_eq!(
            check_declared_cost_classes([("mystery_rest", None)]).unwrap_err(),
            FenceError::UnknownCostClass {
                source_name: "mystery_rest".into(),
                value: String::new()
            }
        );
        assert_eq!(
            check_declared_cost_classes([("mystery_rest", Some(""))]).unwrap_err(),
            FenceError::UnknownCostClass {
                source_name: "mystery_rest".into(),
                value: String::new()
            }
        );
        assert!(matches!(
            check_declared_cost_classes([
                ("coinbase_rest", Some("free")),
                ("polygon_rest", Some("paid")),
            ]),
            Err(FenceError::PaidWithoutAdr { .. })
        ));
    }

    /// The whole declared set, through the guard's own entry point, so the
    /// function CI will call is the function the crate's own tests exercise. A
    /// guard with a private predicate is a second implementation waiting to drift.
    #[test]
    fn the_shipped_declarations_pass_the_guards_own_entry_point() {
        let declared: Vec<(&str, Option<&str>)> = SourceDecl::ALL
            .iter()
            .map(|make| {
                let s = make();
                (s.name, Some(s.cost_class.as_str()))
            })
            .collect();
        assert!(check_declared_cost_classes(declared).is_ok());
    }

    /// P-1.4's acceptance clause, as a test: **two backfilled rows for the same bar
    /// close from different sources carry different knowledge times.** Alpaca's
    /// declared lag is fifteen times Coinbase's, so the two stamps are fifteen
    /// minutes apart — and both are flagged, so neither can be mistaken for an
    /// observation.
    ///
    /// The failure this closes is the one that looks like diligence: treating a
    /// backfilled bar's knowledge time as a property of the *bar* rather than of the
    /// source that handed it over. Do that and a mixed-venue history claims every
    /// Alpaca bar was knowable fourteen minutes before Alpaca published it.
    #[test]
    fn two_backfilled_rows_for_one_bar_close_from_different_sources_differ_in_knowledge_time() {
        let close = t(1);
        let fast = SourceDecl::coinbase_rest()
            .knowledge_for(close, Collection::Historical)
            .unwrap();
        let slow = SourceDecl::alpaca_rest()
            .knowledge_for(close, Collection::Historical)
            .unwrap();
        assert_ne!(
            fast.time(),
            slow.time(),
            "two sources with different declared lags cannot share a knowledge time"
        );
        assert_eq!(slow.time() - fast.time(), chrono::Duration::seconds(840));
        for k in [fast, slow] {
            assert!(k.is_derived(), "a history pull is never an observation");
            assert!(k
                .quality_flags()
                .contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
            assert!(!k.is_forward_evidence());
        }
        // Kraken's bars share Coinbase's 60 s, and that is a fact about the venues
        // rather than an accident worth hiding: the assertion is that the lag comes
        // from the source, not that every source differs.
        assert_eq!(
            SourceDecl::kraken_rest()
                .knowledge_for(close, Collection::Historical)
                .unwrap()
                .time(),
            fast.time()
        );
    }

    /// P-1.6's second clause: a dataset over a mixed window carries the **maximum**
    /// declared lag of its sources. A dataset is only as timely as its slowest
    /// source, and every other summary of the mix understates the lag — which
    /// shortens the embargo, which flatters the result.
    #[test]
    fn a_mixed_window_carries_the_maximum_declared_lag_of_its_sources() {
        let mix = [
            SourceDecl::coinbase_rest(),
            SourceDecl::kraken_rest(),
            SourceDecl::alpaca_rest(),
        ];
        assert_eq!(
            max_declared_lag(mix.iter()),
            Some(Duration::from_secs(900)),
            "the slowest source in the mix, not the commonest and not the mean"
        );
        assert_eq!(max_knowledge_lag_ms(mix.iter()).unwrap(), 900_000);
        // Order cannot change the answer.
        let reversed: Vec<_> = mix.iter().rev().collect();
        assert_eq!(max_knowledge_lag_ms(reversed.into_iter()).unwrap(), 900_000);
        // A single-source dataset carries that source's lag, unchanged.
        assert_eq!(
            max_knowledge_lag_ms([SourceDecl::coinbase_rest()].iter()).unwrap(),
            60_000
        );
    }

    /// Adversarial: a dataset that names no source is refused rather than handed a
    /// zero lag. `max_knowledge_lag_ms = 0` means "nothing was ever late", which is
    /// the most optimistic possible answer and the one an unresolved source list
    /// would silently produce.
    #[test]
    fn a_dataset_that_names_no_source_is_refused_rather_than_given_a_zero_lag() {
        let empty: [SourceDecl; 0] = [];
        assert_eq!(max_declared_lag(empty.iter()), None);
        assert_eq!(max_knowledge_lag_ms(empty.iter()), Err(LagError::NoSources));
    }

    /// And the figure reaches the embargo: a mix containing Alpaca embargoes more
    /// bars than one that does not. Without this the maximum would be a number
    /// computed and then dropped.
    #[test]
    fn the_maximum_lag_lengthens_the_embargo_it_is_computed_for() {
        let inputs = |lag_ms: u64| crate::split::EmbargoInputs {
            horizon_bars: 5,
            max_lookback_bars: 10,
            max_knowledge_lag_ms: lag_ms,
            settlement_lag_bars: 0,
        };
        let crypto_only = max_knowledge_lag_ms([SourceDecl::coinbase_rest()].iter()).unwrap();
        let mixed =
            max_knowledge_lag_ms([SourceDecl::coinbase_rest(), SourceDecl::alpaca_rest()].iter())
                .unwrap();
        let narrow = crate::split::compute_embargo(&inputs(crypto_only));
        let wide = crate::split::compute_embargo(&inputs(mixed));
        assert!(
            wide > narrow,
            "a window that includes a 900 s source must embargo more than one that does not \
             ({wide} vs {narrow})"
        );
        assert_eq!(wide - narrow, 14, "15 minutes minus 1 minute, in 1m bars");
    }
}
