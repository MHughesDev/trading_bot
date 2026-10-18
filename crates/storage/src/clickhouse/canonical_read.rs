//! A point-in-time read of `market_bar` that keeps each bar's provenance (Set P
//! P-4.1, P-4.2).
//!
//! `dataplane::resample` and `dataplane::bars::information_driven` both require a
//! `SourceProvenance` for every distinct `source_id` in their input, and refuse a
//! source with no grade — so a derived series can never report itself cleaner than
//! what it came from. `backtest::store::BarStore::load_bars` returns `LoadedBar`,
//! which carries neither `source_id` nor `vwap`, so a route built on it would have
//! had to invent a source and a grade: the laundering step those refusals exist to
//! prevent. This read returns `CanonicalBar` with both.
//!
//! **Point in time, the same way `BarStore` resolves it.** Only rows with
//! `knowledge_time <= as_of` are seen, and per bar the row with the greatest
//! `(knowledge_time, revision_seq)` wins *as a whole row* — every column comes
//! from the same revision, via `argMax(tuple(…))`, so a restatement can never be
//! assembled out of one revision's high and another's low.
//!
//! Provenance — each source's declared lag from `dataplane.source` and the input
//! series' grade from `data_qc_reports` — is looked up, never assumed. A series
//! nobody has graded comes back without a grade, and the derivation refuses it.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use clickhouse::Row;
use dataplane::bar::CanonicalBar;
use dataplane::identity::{InstrumentKey, VenueKey};
use dataplane::qc::Grade;
use dataplane::quality::QualityFlags;
use dataplane::resample::SourceProvenance;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{connect, ChError};

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// One bar's winning revision: knowledge_ns, ingest_ns, open, high, low, close,
/// volume, trade_count, vwap, quality_flags, revision_seq, source_id.
type Revision = (
    i64,
    i64,
    String,
    String,
    String,
    String,
    String,
    Option<u32>,
    Option<String>,
    u32,
    u32,
    i32,
);

#[derive(Row, Deserialize)]
struct Wire {
    instrument_id: i64,
    venue_id: i32,
    event_ns: i64,
    r: Revision,
}

fn dec(s: &str) -> Result<Decimal, ChError> {
    s.parse()
        .map_err(|e| ChError::Decode(format!("decimal {s:?}: {e}")))
}

/// Bars of one `(symbol, venue, period)` with opens in `[from, to)`, as known at
/// `as_of`, in event-time order, one row per bar.
///
/// # Errors
/// The backend, or a stored value that will not decode.
pub async fn canonical_bars_as_of(
    url: &str,
    symbol: &str,
    venue: &str,
    period_secs: u32,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> Result<Vec<CanonicalBar>, ChError> {
    let ns = |t: DateTime<Utc>| t.timestamp_nanos_opt().unwrap_or(i64::MAX);
    let rows: Vec<Wire> = connect(url)
        .query(&format!(
            "SELECT b.instrument_id AS instrument_id, b.venue_id AS venue_id,
                    toInt64(toUnixTimestamp64Nano(b.event_time)) AS event_ns,
                    argMax(tuple(
                        toInt64(toUnixTimestamp64Nano(b.knowledge_time)),
                        toInt64(toUnixTimestamp64Nano(b.ingest_time)),
                        toString(b.open), toString(b.high), toString(b.low),
                        toString(b.close), toString(b.volume),
                        b.trade_count, toString(b.vwap),
                        b.quality_flags, b.revision_seq, b.source_id),
                      (b.knowledge_time, b.revision_seq)) AS r
               FROM market_bar AS b
              WHERE b.instrument_id IN (SELECT instrument_id FROM instrument_symbol_dim WHERE symbol = '{s}')
                AND b.venue_id IN (SELECT venue_id FROM venue_dim WHERE name = '{v}')
                AND b.period_secs = {period_secs}
                AND b.event_time >= fromUnixTimestamp64Nano(toInt64({f}))
                AND b.event_time <  fromUnixTimestamp64Nano(toInt64({t}))
                AND b.knowledge_time <= fromUnixTimestamp64Nano(toInt64({a}))
              GROUP BY b.instrument_id, b.venue_id, b.event_time
              ORDER BY b.event_time",
            s = esc(symbol),
            v = esc(venue),
            f = ns(from),
            t = ns(to),
            a = ns(as_of),
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    rows.into_iter()
        .map(|w| {
            let r = w.r;
            Ok(CanonicalBar {
                instrument_id: InstrumentKey(w.instrument_id),
                venue_id: VenueKey(w.venue_id),
                period_secs,
                event_time: DateTime::from_timestamp_nanos(w.event_ns),
                venue_ts: None,
                ingest_time: DateTime::from_timestamp_nanos(r.1),
                knowledge_time: DateTime::from_timestamp_nanos(r.0),
                open: dec(&r.2)?,
                high: dec(&r.3)?,
                low: dec(&r.4)?,
                close: dec(&r.5)?,
                volume: dec(&r.6)?,
                trade_count: r.7,
                vwap: r.8.as_deref().map(dec).transpose()?,
                bid_close: None,
                ask_close: None,
                quality_flags: QualityFlags(r.9),
                revision_seq: r.10,
                source_id: r.11,
            })
        })
        .collect()
}

/// Provenance for every source in `bars`: the declared lag from
/// `dataplane.source`, and the input series' grade.
///
/// `grade` is the series' persisted grade (`data_qc_reports`), or `None` when it
/// was never graded — in which case **no** provenance is returned for any source,
/// and the derivation refuses with its own named error rather than this function
/// inventing a letter.
///
/// # Errors
/// The database, or a source id with no row in `dataplane.source`.
pub async fn provenance(
    pg: &sqlx::PgPool,
    bars: &[CanonicalBar],
    grade: Option<Grade>,
) -> Result<Vec<SourceProvenance>, ChError> {
    let Some(grade) = grade else {
        return Ok(Vec::new());
    };
    let ids: BTreeSet<i32> = bars.iter().map(|b| b.source_id).collect();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT declared_vendor_lag_ms FROM dataplane.source WHERE source_id = $1",
        )
        .bind(id)
        .fetch_optional(pg)
        .await
        .map_err(|e| ChError::Client(format!("dataplane.source: {e}")))?;
        let (lag_ms,) = row.ok_or_else(|| {
            ChError::Decode(format!(
                "source_id {id} is not in dataplane.source; its lag cannot be declared, so nothing is derived from it"
            ))
        })?;
        out.push(SourceProvenance {
            source_id: id,
            grade,
            declared_lag: Duration::from_millis(u64::try_from(lag_ms).unwrap_or(0)),
        });
    }
    Ok(out)
}

/// Parse a persisted grade letter.
#[must_use]
pub fn parse_grade(s: &str) -> Option<Grade> {
    match s {
        "A" => Some(Grade::A),
        "B" => Some(Grade::B),
        "C" => Some(Grade::C),
        "D" => Some(Grade::D),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_four_letters_parse() {
        assert_eq!(parse_grade("A"), Some(Grade::A));
        assert_eq!(parse_grade("D"), Some(Grade::D));
        assert_eq!(parse_grade("ungraded"), None);
        assert_eq!(parse_grade("a"), None);
    }

    /// No grade, no provenance — so the derivation's own refusal fires.
    #[tokio::test]
    async fn an_ungraded_series_yields_no_provenance_without_touching_the_database() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://nobody@127.0.0.1:1/none")
            .unwrap();
        assert!(provenance(&pool, &[], None).await.unwrap().is_empty());
    }
}
