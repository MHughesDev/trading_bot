//! The source registry and its declared vendor lags (SPEC §1, OQ-01, ADR-P2-24).
//!
//! **Shared file.** Every track that adds a data source adds its rows here and to
//! the matching `dataplane.source` migration, and changes nothing else. The two
//! are one fact written twice: this array is what Rust code reads when it needs a
//! lag without a database round-trip, the table is what SQL joins against.
//!
//! A *declared vendor lag* is how late the source's data is relative to the event
//! it describes. It is the honest floor under `available_time`, and it is what
//! `EmbargoInputs.max_knowledge_lag_ms` accumulates so that a dataset built from
//! a 15-minute-delayed feed cannot pretend to have been real-time.
//!
//! Rows 1–16 mirror migration 0046. Rows 30+ are Set P Phase 3's alternative
//! sources (migrations 0102–0103).

use serde::{Deserialize, Serialize};

/// How a source delivers data. Mirrors the `kind` CHECK in `dataplane.source`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    LiveStream,
    RestHistory,
    VendorFile,
    Derived,
}

impl SourceKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LiveStream => "live_stream",
            Self::RestHistory => "rest_history",
            Self::VendorFile => "vendor_file",
            Self::Derived => "derived",
        }
    }
}

/// One row of `dataplane.source`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceDecl {
    pub source_id: i32,
    pub name: &'static str,
    pub kind: SourceKind,
    /// Milliseconds between the event and the earliest moment the platform could
    /// have held the row. Never negative, and never optimistic.
    pub declared_vendor_lag_ms: i64,
    pub notes: &'static str,
}

use SourceKind::{Derived, LiveStream, RestHistory};

/// Every source the platform reads, in `source_id` order.
pub const SOURCES: &[SourceDecl] = &[
    // ── migration 0046 ────────────────────────────────────────────────────────
    SourceDecl { source_id: 1,  name: "live_aggregator", kind: LiveStream,  declared_vendor_lag_ms: 0,      notes: "trades aggregated to bars in-process" },
    SourceDecl { source_id: 2,  name: "coinbase_rest",   kind: RestHistory, declared_vendor_lag_ms: 60_000,  notes: "candles published after the minute closes" },
    SourceDecl { source_id: 3,  name: "kraken_rest",     kind: RestHistory, declared_vendor_lag_ms: 60_000,  notes: "" },
    SourceDecl { source_id: 4,  name: "binance_rest",    kind: RestHistory, declared_vendor_lag_ms: 60_000,  notes: "" },
    SourceDecl { source_id: 5,  name: "alpaca_rest",     kind: RestHistory, declared_vendor_lag_ms: 900_000, notes: "free-tier SIP data is 15-minute delayed" },
    SourceDecl { source_id: 6,  name: "unknown_legacy",  kind: RestHistory, declared_vendor_lag_ms: 60_000,  notes: "pre-ledger rows with no recorded source" },
    SourceDecl { source_id: 7,  name: "tradier_rest",    kind: RestHistory, declared_vendor_lag_ms: 900_000, notes: "delayed options quotes" },
    SourceDecl { source_id: 8,  name: "onchain_indexer", kind: LiveStream,  declared_vendor_lag_ms: 0,      notes: "knowledge_time is when the indexer saw the block" },
    SourceDecl { source_id: 9,  name: "kraken_ws",       kind: LiveStream,  declared_vendor_lag_ms: 0,      notes: "" },
    SourceDecl { source_id: 10, name: "synthetic",       kind: Derived,     declared_vendor_lag_ms: 0,      notes: "generated test series; never tradeable" },
    SourceDecl { source_id: 11, name: "alpaca_ws",       kind: LiveStream,  declared_vendor_lag_ms: 0,      notes: "" },
    SourceDecl { source_id: 12, name: "oanda_rest",      kind: RestHistory, declared_vendor_lag_ms: 5_000,   notes: "" },
    SourceDecl { source_id: 13, name: "kalshi_rest",     kind: RestHistory, declared_vendor_lag_ms: 60_000,  notes: "" },
    SourceDecl { source_id: 14, name: "tradovate_rest",  kind: RestHistory, declared_vendor_lag_ms: 60_000,  notes: "" },
    SourceDecl { source_id: 15, name: "zerox_rest",      kind: RestHistory, declared_vendor_lag_ms: 30_000,  notes: "DEX quotes via 0x" },
    SourceDecl { source_id: 16, name: "e2e_test",        kind: Derived,     declared_vendor_lag_ms: 0,      notes: "integration-test fixture source" },

    // ── Set P Phase 3, alternative data (migrations 0102–0103) ────────────────
    //
    // ALFRED serves a vintage the day it is published, but not the minute. The
    // lag is one day so that a vintage dated D is not treated as knowable before
    // D ends — see `vintage::derived_knowledge_time`.
    SourceDecl { source_id: 30, name: "alfred_rest",     kind: RestHistory, declared_vendor_lag_ms: 86_400_000, notes: "FRED/ALFRED vintages; vintage_date is a date, so the lag covers the release day" },
    // EDGAR publishes an acceptance timestamp per submission, so the observed
    // knowledge time is exact. The SEC states the submissions API is updated in
    // real time with "a typical processing delay of less than a second"; one
    // minute is that with a margin, not a guess.
    SourceDecl { source_id: 31, name: "sec_edgar",       kind: RestHistory, declared_vendor_lag_ms: 60_000,     notes: "acceptance timestamp observed; lag covers dissemination (SEC states sub-second typical)" },
    SourceDecl { source_id: 32, name: "alpaca_news",     kind: RestHistory, declared_vendor_lag_ms: 0,          notes: "news carries its own publication time; knowledge time is observed" },
    SourceDecl { source_id: 33, name: "alpaca_corp_actions", kind: RestHistory, declared_vendor_lag_ms: 86_400_000, notes: "corporate-action announcements are dated, not timestamped; one-day lag" },
    // Kalshi *history* through the REST candlesticks endpoint, distinct from the
    // live quote poller (source 13) so a backfilled probability is never confused
    // with an observed one.
    SourceDecl { source_id: 34, name: "kalshi_history",  kind: RestHistory, declared_vendor_lag_ms: 60_000,     notes: "candlestick history; exposure only, no trading endpoints" },
];

/// Look a source up by its registry name.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static SourceDecl> {
    SOURCES.iter().find(|s| s.name == name)
}

/// Look a source up by id.
#[must_use]
pub fn by_id(source_id: i32) -> Option<&'static SourceDecl> {
    SOURCES.iter().find(|s| s.source_id == source_id)
}

/// The declared lag of `name`, in milliseconds.
///
/// # Errors
/// An unregistered source has no declared lag, and guessing one is how a dataset
/// comes to claim it was more timely than it was.
pub fn declared_lag_ms(name: &str) -> Result<i64, UnknownSource> {
    by_name(name)
        .map(|s| s.declared_vendor_lag_ms)
        .ok_or_else(|| UnknownSource(name.to_owned()))
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "source {0:?} is not in the registry — declare it in dataplane::source::SOURCES and in a \
     dataplane.source migration before reading from it"
)]
pub struct UnknownSource(pub String);

/// The largest declared lag across `names` — what a dataset drawing on several
/// sources must carry into `EmbargoInputs.max_knowledge_lag_ms`.
///
/// # Errors
/// Any unregistered name.
pub fn max_declared_lag_ms<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<i64, UnknownSource> {
    let mut max = 0i64;
    for n in names {
        max = max.max(declared_lag_ms(n)?);
    }
    Ok(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_and_names_are_unique() {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for s in SOURCES {
            assert!(ids.insert(s.source_id), "duplicate source_id {}", s.source_id);
            assert!(names.insert(s.name), "duplicate source name {}", s.name);
        }
    }

    #[test]
    fn no_source_declares_a_negative_lag() {
        for s in SOURCES {
            assert!(
                s.declared_vendor_lag_ms >= 0,
                "{} declares a negative lag",
                s.name
            );
        }
    }

    /// An unregistered source has no lag, rather than a default of zero. Zero is
    /// the most flattering possible answer and would be silently wrong.
    #[test]
    fn an_unregistered_source_has_no_lag() {
        assert!(declared_lag_ms("bloomberg").is_err());
        assert!(max_declared_lag_ms(["alfred_rest", "bloomberg"]).is_err());
    }

    #[test]
    fn the_maximum_lag_is_what_a_multi_source_dataset_carries() {
        // A dataset over bars and macro inherits the macro release day, not the
        // bar's minute.
        assert_eq!(
            max_declared_lag_ms(["coinbase_rest", "alfred_rest"]).expect("both registered"),
            86_400_000
        );
    }

    #[test]
    fn phase_3_sources_are_registered() {
        for name in [
            "alfred_rest",
            "sec_edgar",
            "alpaca_news",
            "alpaca_corp_actions",
            "kalshi_history",
        ] {
            assert!(by_name(name).is_some(), "{name} missing from the registry");
        }
    }
}
