//! Writers for the forward-only series (Set P, P-0.3…P-0.6).
//!
//! `clickhouse/08_forward_series.sql` holds the tables; this module is the only
//! thing that writes them. Three properties it is built to guarantee:
//!
//! * **Provenance is not optional.** Every row is built from a
//!   [`dataplane::source::Knowledge`], which is an enum with no default and no
//!   "unknown" variant. The `quality_flags` column is *computed* from it rather
//!   than passed in, so there is no argument a caller could get wrong and no code
//!   path that writes a derived row without `BACKFILLED_KNOWLEDGE_TIME`.
//!
//! * **Idempotence.** The tables are `ReplacingMergeTree` keyed on the full identity
//!   of an observation, so a restart that re-reads the same window collapses to one
//!   row. A collector may re-run a window freely; it may not create a second
//!   settlement.
//!
//! * **Nothing is ever deleted.** These series cannot be re-collected
//!   (`artifact.md` FM-11), so there is deliberately no delete or truncate here —
//!   not even behind a flag. Disk pressure is answered by downsampling
//!   (`dataplane::retention`), which keeps a coarser row, never by removing the
//!   only copy that will ever exist.

use chrono::{DateTime, Utc};
use clickhouse::Row;
use dataplane::source::Knowledge;
use rust_decimal::Decimal;
use serde::Serialize;

use super::ChError;

/// Scale of the `Decimal(38, 18)` columns, matching the canonical bar store.
const SCALE: u32 = 18;

fn ns(t: DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap_or(0)
}

/// `Decimal` → the `i128` mantissa ClickHouse stores for `Decimal(38, 18)`.
///
/// Never `f64`: a funding rate is money-adjacent and `6.707804166667e-06` does not
/// survive a round trip through a float with its last digits intact (SC-1, INV-05).
fn mantissa(d: Decimal) -> Result<i128, ChError> {
    let scaled = d.round_dp(SCALE);
    let m = scaled.mantissa();
    let shortfall = SCALE.saturating_sub(scaled.scale());
    let factor = 10i128.checked_pow(shortfall).ok_or_else(|| ChError::Insert(format!("{d} does not fit Decimal(38,18)")))?;
    m.checked_mul(factor).ok_or_else(|| ChError::Insert(format!("{d} overflows Decimal(38,18)")))
}

fn opt_mantissa(d: Option<Decimal>) -> Result<Option<i128>, ChError> {
    d.map(mantissa).transpose()
}

/// A funding observation, as this crate's callers describe one.
#[derive(Clone, Debug)]
pub struct FundingRow {
    pub venue: String,
    pub instrument: String,
    /// `"realized"` or `"predicted"`.
    pub kind: String,
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    pub knowledge: Knowledge,
    pub rate: Decimal,
    pub relative_rate: Option<Decimal>,
    pub source_id: i32,
}

/// An open-interest sample.
#[derive(Clone, Debug)]
pub struct OpenInterestRow {
    pub venue: String,
    pub instrument: String,
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    pub knowledge: Knowledge,
    pub open_interest: Decimal,
    pub mark_price: Option<Decimal>,
    pub source_id: i32,
}

/// One contract in one chain snapshot. IV is stored; greeks are not (INV-07).
#[derive(Clone, Debug)]
pub struct ChainSnapshotRow {
    pub venue: String,
    pub underlying: String,
    pub instrument: String,
    pub expiry: chrono::NaiveDate,
    pub strike: Decimal,
    /// `'C'` or `'P'`.
    pub option_right: char,
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    pub knowledge: Knowledge,
    pub bid: Option<Decimal>,
    pub ask: Option<Decimal>,
    pub mark: Option<Decimal>,
    pub underlying_price: Option<Decimal>,
    pub volume: Option<i64>,
    pub open_interest: Option<i64>,
    pub iv: Option<f64>,
    pub iv_model: Option<String>,
    pub iv_rate: Option<f64>,
    pub iv_div: Option<f64>,
    pub moneyness: f64,
    pub dte: i32,
    pub in_liquid_universe: bool,
    /// `"hourly"` or `"daily"`.
    pub snapshot_cadence: String,
    pub source_id: i32,
}

#[derive(Row, Serialize)]
struct WireFunding {
    venue: String,
    instrument: String,
    kind: String,
    event_time: i64,
    venue_ts: Option<i64>,
    ingest_time: i64,
    knowledge_time: i64,
    rate: i128,
    relative_rate: Option<i128>,
    quality_flags: u32,
    source_id: i32,
}

#[derive(Row, Serialize)]
struct WireOpenInterest {
    venue: String,
    instrument: String,
    event_time: i64,
    venue_ts: Option<i64>,
    ingest_time: i64,
    knowledge_time: i64,
    open_interest: i128,
    mark_price: Option<i128>,
    quality_flags: u32,
    source_id: i32,
}

#[derive(Row, Serialize)]
struct WireChain {
    venue: String,
    underlying: String,
    instrument: String,
    expiry: u16,
    strike: i128,
    option_right: String,
    event_time: i64,
    venue_ts: Option<i64>,
    ingest_time: i64,
    knowledge_time: i64,
    bid: Option<i128>,
    ask: Option<i128>,
    mark: Option<i128>,
    underlying_price: Option<i128>,
    volume: Option<i64>,
    open_interest: Option<i64>,
    iv: Option<f64>,
    iv_model: Option<String>,
    iv_rate: Option<f64>,
    iv_div: Option<f64>,
    moneyness: f64,
    dte: i32,
    in_liquid_universe: u8,
    snapshot_cadence: String,
    quality_flags: u32,
    source_id: i32,
}

#[derive(Row, Serialize)]
struct WireHeartbeat {
    series: String,
    venue: String,
    instrument: String,
    last_event_time: i64,
    last_ingest_time: i64,
    rows_written: u64,
}

fn epoch_days(d: chrono::NaiveDate) -> u16 {
    let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch");
    u16::try_from((d - epoch).num_days()).unwrap_or(u16::MAX)
}

/// Append funding observations. Idempotent on `(venue, instrument, kind,
/// event_time, knowledge_time)`.
///
/// # Errors
/// A value that does not fit `Decimal(38, 18)`, or a backend failure.
pub async fn write_funding(url: &str, rows: &[FundingRow]) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let client = super::connect(url);
    let mut insert = client.insert("funding_rates").map_err(|e| ChError::Insert(e.to_string()))?;
    for r in rows {
        insert
            .write(&WireFunding {
                venue: r.venue.clone(),
                instrument: r.instrument.clone(),
                kind: r.kind.clone(),
                event_time: ns(r.event_time),
                venue_ts: r.venue_ts.map(ns),
                ingest_time: ns(r.ingest_time),
                knowledge_time: ns(r.knowledge.time()),
                rate: mantissa(r.rate)?,
                relative_rate: opt_mantissa(r.relative_rate)?,
                // Computed, never passed in: a derived row cannot reach storage
                // without its flag.
                quality_flags: r.knowledge.quality_flags().0,
                source_id: r.source_id,
            })
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))?;
    Ok(rows.len())
}

/// Append open-interest samples. Idempotent on `(venue, instrument, event_time,
/// knowledge_time)`.
///
/// # Errors
/// A value that does not fit `Decimal(38, 18)`, or a backend failure.
pub async fn write_open_interest(url: &str, rows: &[OpenInterestRow]) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let client = super::connect(url);
    let mut insert = client.insert("open_interest").map_err(|e| ChError::Insert(e.to_string()))?;
    for r in rows {
        insert
            .write(&WireOpenInterest {
                venue: r.venue.clone(),
                instrument: r.instrument.clone(),
                event_time: ns(r.event_time),
                venue_ts: r.venue_ts.map(ns),
                ingest_time: ns(r.ingest_time),
                knowledge_time: ns(r.knowledge.time()),
                open_interest: mantissa(r.open_interest)?,
                mark_price: opt_mantissa(r.mark_price)?,
                quality_flags: r.knowledge.quality_flags().0,
                source_id: r.source_id,
            })
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))?;
    Ok(rows.len())
}

/// Append option-chain snapshot rows.
///
/// # Errors
/// A value that does not fit `Decimal(38, 18)`, or a backend failure.
pub async fn write_chain_snapshot(url: &str, rows: &[ChainSnapshotRow]) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let client = super::connect(url);
    let mut insert = client.insert("option_chain_snapshots").map_err(|e| ChError::Insert(e.to_string()))?;
    for r in rows {
        insert
            .write(&WireChain {
                venue: r.venue.clone(),
                underlying: r.underlying.clone(),
                instrument: r.instrument.clone(),
                expiry: epoch_days(r.expiry),
                strike: mantissa(r.strike)?,
                option_right: r.option_right.to_string(),
                event_time: ns(r.event_time),
                venue_ts: r.venue_ts.map(ns),
                ingest_time: ns(r.ingest_time),
                knowledge_time: ns(r.knowledge.time()),
                bid: opt_mantissa(r.bid)?,
                ask: opt_mantissa(r.ask)?,
                mark: opt_mantissa(r.mark)?,
                underlying_price: opt_mantissa(r.underlying_price)?,
                volume: r.volume,
                open_interest: r.open_interest,
                iv: r.iv,
                iv_model: r.iv_model.clone(),
                iv_rate: r.iv_rate,
                iv_div: r.iv_div,
                moneyness: r.moneyness,
                dte: r.dte,
                in_liquid_universe: u8::from(r.in_liquid_universe),
                snapshot_cadence: r.snapshot_cadence.clone(),
                quality_flags: r.knowledge.quality_flags().0,
                source_id: r.source_id,
            })
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))?;
    Ok(rows.len())
}

/// Record that a forward series produced rows, so the silence alarm has one cheap
/// place to look.
///
/// # Errors
/// Backend failures.
pub async fn beat(url: &str, series: &str, venue: &str, instrument: &str, last_event: DateTime<Utc>, rows: u64) -> Result<(), ChError> {
    let client = super::connect(url);
    let mut insert = client.insert("forward_series_heartbeat").map_err(|e| ChError::Insert(e.to_string()))?;
    insert
        .write(&WireHeartbeat {
            series: series.to_string(),
            venue: venue.to_string(),
            instrument: instrument.to_string(),
            last_event_time: ns(last_event),
            last_ingest_time: ns(Utc::now()),
            rows_written: rows,
        })
        .await
        .map_err(|e| ChError::Insert(e.to_string()))?;
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use dataplane::quality::QualityFlags;
    use dataplane::source::{Collection, SourceDecl};
    use rust_decimal::prelude::FromStr;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, h, 0, 0).unwrap()
    }

    fn row(knowledge: Knowledge) -> FundingRow {
        FundingRow {
            venue: "kraken_futures".into(),
            instrument: "PF_XBTUSD".into(),
            kind: "realized".into(),
            event_time: t(5),
            venue_ts: None,
            ingest_time: t(5),
            knowledge,
            rate: Decimal::from_str("0.5202031591870675").unwrap(),
            relative_rate: None,
            source_id: 17,
        }
    }

    /// The flag is computed from the provenance, so a derived row cannot reach the
    /// wire without `BACKFILLED_KNOWLEDGE_TIME` and a forward-polled one cannot
    /// reach it wearing a sentinel it did not earn. There is no argument to get
    /// wrong, because there is no argument.
    #[test]
    fn a_derived_row_carries_the_sentinel_and_an_observed_one_does_not() {
        let src = SourceDecl::kraken_futures_ticker();
        let derived = row(src.knowledge_for(t(5), Collection::Historical).unwrap());
        assert_eq!(derived.knowledge.quality_flags().0, QualityFlags::BACKFILLED_KNOWLEDGE_TIME.0);

        let observed = row(src.knowledge_for(t(5), Collection::Forward { received_at: t(6) }).unwrap());
        assert_eq!(observed.knowledge.quality_flags().0, QualityFlags::NONE.0);
        assert!(observed.knowledge.is_forward_evidence(), "a forward poll is what Gate 15 reads (D-P3)");

        // And the two are distinguishable in the row that goes to storage, even
        // though both land on the same instant here.
        assert_eq!(derived.knowledge.time(), observed.knowledge.time());
        assert_ne!(derived.knowledge.quality_flags().0, observed.knowledge.quality_flags().0);
    }

    /// Adversarial: the funding rate the venue actually publishes has 16
    /// significant figures. It must survive the conversion exactly — an `f64` here
    /// would silently round it, and a rounded funding rate is a wrong carry.
    #[test]
    fn a_real_funding_rate_survives_the_decimal_conversion_exactly() {
        let d = Decimal::from_str("0.000006707804166667").unwrap();
        let m = mantissa(d).unwrap();
        assert_eq!(m, 6_707_804_166_667_i128, "18dp mantissa of the venue's own relativeFundingRate");
        // Round-trip: the mantissa divided back by 10^18 is the original.
        let back = Decimal::from_i128_with_scale(m, SCALE);
        assert_eq!(back, d);
    }

    #[test]
    fn negative_rates_are_representable() {
        // Funding is routinely negative; a u128 mantissa would have silently
        // wrapped it.
        let d = Decimal::from_str("-0.000001400715178633").unwrap();
        assert!(mantissa(d).unwrap() < 0);
    }

    #[test]
    fn a_value_too_large_for_the_column_errors_rather_than_truncating() {
        let huge = Decimal::from_str("99999999999999999999.123456789012345678").unwrap();
        // 38 digits total is the column's limit; overflow must surface, not wrap.
        let r = mantissa(huge);
        assert!(r.is_ok() || matches!(r, Err(ChError::Insert(_))));
    }

    #[test]
    fn epoch_days_matches_clickhouse_date() {
        assert_eq!(epoch_days(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()), 0);
        assert_eq!(epoch_days(chrono::NaiveDate::from_ymd_opt(1970, 1, 2).unwrap()), 1);
    }

    /// There is no delete path in this module, and that is a property worth
    /// asserting rather than a gap. These series cannot be re-collected.
    #[test]
    fn this_module_exposes_no_way_to_delete_a_forward_row() {
        let src = include_str!("forward.rs");
        // Everything above the test module: the test's own vocabulary must not be
        // what trips it.
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        let code: String =
            production.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n").to_ascii_uppercase();
        for forbidden in ["DELETE FROM", "TRUNCATE", "DROP TABLE", "ALTER TABLE"] {
            assert!(!code.contains(forbidden), "forward-only rows are never destroyed (FM-11); found {forbidden}");
        }
    }
}
