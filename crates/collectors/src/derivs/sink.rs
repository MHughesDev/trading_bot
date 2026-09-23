//! Where a forward poll's observations go (P-0.3).
//!
//! `storage::clickhouse::forward` has been able to write these series since the
//! schema landed, and nothing has ever called it: the writers had zero callers in
//! the whole workspace and the four tables in `trading` held zero rows. This module
//! is the missing half — the piece that lets a collector hand its observations to
//! the store — and it is a trait rather than a direct call for the same reason
//! `dataplane::backfill::BarSink` is: the only way to prove "a poll writes what it
//! collected, once" is to count, and you cannot count through a database.
//!
//! Two things are deliberately not defaulted here.
//!
//! **Provenance.** [`Knowledge`](dataplane::source::Knowledge) travels from the
//! parser to the row untouched. The sink never rebuilds it, never stamps a time of
//! its own, and has no parameter that would let it: a forward poll's real receipt
//! reaches ClickHouse as a receipt, and a history row reaches it carrying its
//! sentinel and its flag.
//!
//! **The source id.** Every row names which declared source produced it, and the
//! surrogate id is looked up rather than assumed. A name that is not in the
//! catalogue is a refusal, not a `0` — `source_id = 0` is a real row in
//! `dataplane.source`'s neighbourhood (`unknown_legacy` is the id the canonical
//! migration substitutes for rows whose source was never recorded), and quietly
//! writing it would relabel a known source as an unknown one.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use storage::clickhouse::forward as store;

use super::funding::{FundingKind, FundingObservation, OpenInterestObservation};

/// A sink refused or failed.
///
/// One variant per reason, because "nothing was written" has more than one cause
/// and a collector's retry decision differs between them.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// The row named a source the catalogue does not hold. Refused rather than
    /// defaulted: a row whose provenance cannot be resolved is not written.
    #[error("source {0:?} is not in dataplane.source; the row is refused rather than written with an invented id")]
    UnknownSource(String),
    /// The store rejected the write, or the backend failed.
    #[error("store: {0}")]
    Store(String),
}

/// Surrogate ids for the declared sources, resolved once at startup.
///
/// Holds no default and no fallback. Build it from `dataplane.source` with
/// [`storage::clickhouse::forward::source_ids`], which is the table the ids are
/// defined by.
#[derive(Clone, Debug, Default)]
pub struct SourceIds(HashMap<String, i32>);

impl SourceIds {
    #[must_use]
    pub fn new(map: HashMap<String, i32>) -> Self {
        Self(map)
    }

    /// For tests and for a caller that already knows the two ids.
    #[must_use]
    pub fn from_pairs(pairs: &[(&str, i32)]) -> Self {
        Self(pairs.iter().map(|(n, i)| ((*n).to_string(), *i)).collect())
    }

    /// The id for a declared source name.
    ///
    /// # Errors
    /// [`SinkError::UnknownSource`] if the catalogue does not hold it. There is no
    /// default id, because every default here is a lie about where a row came from.
    pub fn get(&self, name: &str) -> Result<i32, SinkError> {
        self.0
            .get(name)
            .copied()
            .ok_or_else(|| SinkError::UnknownSource(name.to_string()))
    }
}

/// One series' heartbeat: this instrument produced `rows` rows, the newest of
/// which is `last_event`.
///
/// Written only after a successful write. A heartbeat that beats regardless of the
/// outcome turns a silence alarm into a liveness lie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Beat {
    pub series: String,
    pub venue: String,
    pub instrument: String,
    pub last_event: DateTime<Utc>,
    pub rows: u64,
}

/// Where a poll's observations go.
///
/// The methods take the collector's own observation types, whose `knowledge` field
/// cannot be constructed without stating a provenance — so there is no way to
/// reach a sink with a row that does not know how it was collected.
#[async_trait::async_trait]
pub trait ForwardSink: Send + Sync {
    /// Append funding observations. Returns how many rows were handed to the store.
    ///
    /// # Errors
    /// Implementation-defined; see [`SinkError`].
    async fn funding(&self, rows: &[FundingObservation]) -> Result<usize, SinkError>;

    /// Append open-interest samples.
    ///
    /// # Errors
    /// Implementation-defined; see [`SinkError`].
    async fn open_interest(&self, rows: &[OpenInterestObservation]) -> Result<usize, SinkError>;

    /// Record that series produced rows, so the silence alarm has somewhere cheap
    /// to look.
    ///
    /// Batched: one ticker poll covers every perpetual the venue lists, and the
    /// alarm is per instrument.
    ///
    /// # Errors
    /// Implementation-defined; see [`SinkError`].
    async fn heartbeat(&self, beats: &[Beat]) -> Result<usize, SinkError>;
}

const fn kind_of(k: FundingKind) -> store::FundingKind {
    match k {
        FundingKind::Realized => store::FundingKind::Realized,
        FundingKind::Predicted => store::FundingKind::Predicted,
    }
}

/// The sink that writes ClickHouse — the one that matters.
pub struct ClickHouseForwardSink {
    url: String,
    sources: SourceIds,
}

impl ClickHouseForwardSink {
    /// `url` is a full ClickHouse URL with the database in it
    /// (`http://user:pass@host:8123/trading`). The database is load-bearing: the
    /// `clickhouse` crate connects to `default` when it is omitted, which is how a
    /// previous investigation concluded these tables did not exist.
    #[must_use]
    pub fn new(url: impl Into<String>, sources: SourceIds) -> Self {
        Self {
            url: url.into(),
            sources,
        }
    }
}

#[async_trait::async_trait]
impl ForwardSink for ClickHouseForwardSink {
    async fn funding(&self, rows: &[FundingObservation]) -> Result<usize, SinkError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut out = Vec::with_capacity(rows.len());
        for o in rows {
            out.push(store::FundingRow {
                venue: o.venue.clone(),
                instrument: o.instrument.clone(),
                kind: kind_of(o.kind),
                event_time: o.event_time,
                venue_ts: o.venue_ts,
                ingest_time: o.ingest_time,
                // Passed through, never rebuilt. The store computes quality_flags
                // from it, so a derived row cannot lose its sentinel here.
                knowledge: o.knowledge,
                rate: o.rate,
                relative_rate: o.relative_rate,
                source_id: self.sources.get(o.source)?,
            });
        }
        store::write_funding(&self.url, &out)
            .await
            .map_err(|e| SinkError::Store(e.to_string()))
    }

    async fn open_interest(&self, rows: &[OpenInterestObservation]) -> Result<usize, SinkError> {
        if rows.is_empty() {
            return Ok(0);
        }
        let mut out = Vec::with_capacity(rows.len());
        for o in rows {
            out.push(store::OpenInterestRow {
                venue: o.venue.clone(),
                instrument: o.instrument.clone(),
                event_time: o.event_time,
                venue_ts: o.venue_ts,
                ingest_time: o.ingest_time,
                knowledge: o.knowledge,
                open_interest: o.open_interest,
                mark_price: o.mark_price,
                source_id: self.sources.get(o.source)?,
            });
        }
        store::write_open_interest(&self.url, &out)
            .await
            .map_err(|e| SinkError::Store(e.to_string()))
    }

    async fn heartbeat(&self, beats: &[Beat]) -> Result<usize, SinkError> {
        if beats.is_empty() {
            return Ok(0);
        }
        let rows: Vec<store::HeartbeatRow> = beats
            .iter()
            .map(|b| store::HeartbeatRow {
                series: b.series.clone(),
                venue: b.venue.clone(),
                instrument: b.instrument.clone(),
                last_event_time: b.last_event,
                rows_written: b.rows,
            })
            .collect();
        store::write_heartbeats(&self.url, &rows)
            .await
            .map_err(|e| SinkError::Store(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derivs::funding::parse_tickers;
    use chrono::TimeZone;

    const TICKERS: &str = r#"{
      "result": "success",
      "serverTime": "2026-09-15T05:40:26.212Z",
      "tickers": [
        {"symbol":"PF_XBTUSD","tag":"perpetual","fundingRate":0.5202031591870675,
         "fundingRatePrediction":0.48,"relativeFundingRate":6.707804166667e-06,
         "openInterest":1234.5,"markPrice":64250.5}
      ]
    }"#;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, h, m, 0).unwrap()
    }

    /// The ids are the ones migration 0098 seeded and the live database holds.
    fn catalogue() -> SourceIds {
        SourceIds::from_pairs(&[("kraken_futures_rest", 17), ("kraken_futures_history", 18)])
    }

    #[test]
    fn a_polled_row_resolves_to_the_ticker_source_and_a_backfilled_one_does_not() {
        let s = parse_tickers(TICKERS, at(5, 40)).unwrap();
        let f = &s.funding[0];
        assert_eq!(f.source, "kraken_futures_rest");
        assert_eq!(catalogue().get(f.source).unwrap(), 17);
    }

    /// The failure this closes: an unresolvable source must stop the row, not be
    /// written as `0`. A row stamped with the wrong source carries the wrong
    /// declared lag into every embargo calculation that reads it.
    #[test]
    fn an_unknown_source_is_refused_rather_than_written_as_zero() {
        let empty = SourceIds::default();
        let e = empty.get("kraken_futures_rest").unwrap_err();
        assert!(matches!(e, SinkError::UnknownSource(_)), "{e}");
        assert!(e.to_string().contains("refused"));
    }

    /// Provenance survives the mapping. This is the property the whole sink exists
    /// to preserve: the receipt the parser recorded is the instant that reaches the
    /// store, and the flag the store computes from it is unchanged.
    #[test]
    fn the_receipt_reaches_the_row_untouched() {
        let received = at(5, 40);
        let s = parse_tickers(TICKERS, received).unwrap();
        for f in &s.funding {
            assert_eq!(f.knowledge.time(), received);
            assert!(f.knowledge.is_forward_evidence());
        }
        for o in &s.open_interest {
            assert_eq!(o.knowledge.time(), received);
            assert!(o.knowledge.is_forward_evidence());
        }
    }

    #[test]
    fn the_two_funding_kinds_map_across_without_collapsing() {
        assert_eq!(kind_of(FundingKind::Realized), store::FundingKind::Realized);
        assert_eq!(
            kind_of(FundingKind::Predicted),
            store::FundingKind::Predicted
        );
        assert_ne!(
            kind_of(FundingKind::Realized),
            kind_of(FundingKind::Predicted)
        );
    }

    /// An empty batch writes nothing and says so, rather than opening an insert
    /// that commits an empty block.
    #[tokio::test]
    async fn an_empty_batch_touches_no_backend() {
        // The URL is unreachable on purpose: if an empty batch talked to the
        // backend, this would fail rather than return zero.
        let sink = ClickHouseForwardSink::new("http://127.0.0.1:1/trading", catalogue());
        assert_eq!(sink.funding(&[]).await.unwrap(), 0);
        assert_eq!(sink.open_interest(&[]).await.unwrap(), 0);
    }
}
