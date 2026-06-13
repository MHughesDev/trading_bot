//! `ClickHouse` access for the backtesting system.
//!
//! Reads and writes the canonical `market_bars` table (see
//! `clickhouse/02_bars.sql`).  Reads deduplicate revisions with `argMax` so
//! late-data corrections win, matching the `ReplacingMergeTree` semantics.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use clickhouse::Row;
use domain::payloads::bar::Timeframe;
use rust_decimal::Decimal;
use serde::Deserialize;
use uuid::Uuid;

use crate::types::TimeframeExt;

/// A deduplicated bar loaded from `ClickHouse`.
#[derive(Clone, Debug)]
pub struct LoadedBar {
    /// `available_time` in Unix nanoseconds.
    pub ts_ns: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub trade_count: u64,
}

/// A bar produced by the historical collection system, ready for insert.
#[derive(Clone, Debug)]
pub struct CollectedBar {
    /// Bar close time (becomes `available_time`).
    pub available_time: DateTime<Utc>,
    /// Monotonic sequence for the dedup key (bar open in epoch units).
    pub sequence: u64,
    /// Decimal strings — never floats.
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub trade_count: u64,
}

pub struct BarStore {
    client: clickhouse::Client,
}

#[derive(Row, Deserialize)]
struct DailyCountRow {
    day: String,
    bars: u64,
}

#[derive(Row, Deserialize)]
struct BarRowOut {
    ts_ns: i64,
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
    trade_count: u64,
}

/// Row written to `market_bars` (canonical envelope schema, `clickhouse/02_bars.sql`).
///
/// Columns are serialized through the `clickhouse` crate's typed `RowBinary`
/// path — no SQL string is ever built by hand.  Decimal128 columns are sent as
/// their string form (the repo-wide convention, mirroring
/// `storage::clickhouse::bars`); the two `DateTime64(9, 'UTC')` columns are sent
/// as their `i64` nanosecond tick value, which is exactly the `RowBinary` wire
/// representation of `DateTime64(9)`.
#[derive(Row, serde::Serialize)]
struct CollectedBarRow {
    #[serde(with = "clickhouse::serde::uuid")]
    event_id: Uuid,
    lane: String,
    instrument_id: String,
    venue_id: String,
    source: String,
    trust_tier: String,
    /// DateTime64(9, 'UTC') — Int64 ticks (nanoseconds) since the Unix epoch.
    available_time: i64,
    ingested_time: i64,
    sequence: u64,
    timeframe: String,
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
    trade_count: u64,
    revision: u32,
    dedup_key: String,
}

impl BarStore {
    pub fn connect(url: &str) -> Self {
        Self {
            client: clickhouse::Client::default().with_url(url),
        }
    }

    /// Per-day distinct bar counts for one instrument + timeframe.
    pub async fn daily_counts(
        &self,
        instrument_id: &str,
        timeframe: Timeframe,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<HashMap<NaiveDate, u64>> {
        let rows: Vec<DailyCountRow> = self
            .client
            .query(
                "SELECT toString(toDate(available_time)) AS day, \
                        uniqExact(available_time) AS bars \
                 FROM market_bars \
                 WHERE instrument_id = ? AND timeframe = ? \
                   AND available_time >= fromUnixTimestamp64Nano(?) \
                   AND available_time < fromUnixTimestamp64Nano(?) \
                 GROUP BY day ORDER BY day",
            )
            .bind(instrument_id)
            .bind(timeframe.key())
            .bind(nanos(from))
            .bind(nanos(to))
            .fetch_all()
            .await?;

        let mut out = HashMap::with_capacity(rows.len());
        for row in rows {
            let day: NaiveDate = row.day.parse()?;
            out.insert(day, row.bars);
        }
        Ok(out)
    }

    /// Loads deduplicated bars ordered by `available_time`.
    pub async fn load_bars(
        &self,
        instrument_id: &str,
        timeframe: Timeframe,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<LoadedBar>> {
        let rows: Vec<BarRowOut> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(available_time) AS ts_ns, \
                        argMax(toString(open), revision) AS open, \
                        argMax(toString(high), revision) AS high, \
                        argMax(toString(low), revision) AS low, \
                        argMax(toString(close), revision) AS close, \
                        argMax(toString(volume), revision) AS volume, \
                        argMax(trade_count, revision) AS trade_count \
                 FROM market_bars \
                 WHERE instrument_id = ? AND timeframe = ? \
                   AND available_time >= fromUnixTimestamp64Nano(?) \
                   AND available_time < fromUnixTimestamp64Nano(?) \
                 GROUP BY available_time ORDER BY ts_ns",
            )
            .bind(instrument_id)
            .bind(timeframe.key())
            .bind(nanos(from))
            .bind(nanos(to))
            .fetch_all()
            .await?;

        rows.into_iter()
            .map(|r| {
                Ok(LoadedBar {
                    ts_ns: r.ts_ns,
                    open: r.open.parse()?,
                    high: r.high.parse()?,
                    low: r.low.parse()?,
                    close: r.close.parse()?,
                    volume: r.volume.parse()?,
                    trade_count: r.trade_count,
                })
            })
            .collect()
    }

    /// Inserts collected bars with full envelope columns.
    ///
    /// `event_id` is a deterministic `UUIDv5` of the dedup key, so re-collecting
    /// the same range is idempotent after `ReplacingMergeTree` merges.
    pub async fn insert_collected(
        &self,
        instrument_id: &str,
        venue_id: &str,
        source: &str,
        trust_tier: &str,
        timeframe: Timeframe,
        bars: &[CollectedBar],
    ) -> anyhow::Result<()> {
        if bars.is_empty() {
            return Ok(());
        }
        let lane = format!("market.bars.{}", timeframe.key());
        let ingested_ns = nanos(Utc::now());
        let tf = timeframe.key();

        for chunk in bars.chunks(2_000) {
            let mut insert = self.client.insert("market_bars")?;
            for bar in chunk {
                let dedup_key = format!(
                    "{lane}|{instrument_id}|{venue_id}|{}|{source}",
                    bar.sequence
                );
                let event_id = Uuid::new_v5(&Uuid::NAMESPACE_OID, dedup_key.as_bytes());
                let row = CollectedBarRow {
                    event_id,
                    lane: lane.clone(),
                    instrument_id: instrument_id.to_string(),
                    venue_id: venue_id.to_string(),
                    source: source.to_string(),
                    trust_tier: trust_tier.to_string(),
                    available_time: nanos(bar.available_time),
                    ingested_time: ingested_ns,
                    sequence: bar.sequence,
                    timeframe: tf.to_string(),
                    // `numeric` stays as a defense-in-depth assertion: the
                    // collector boundary must only ever hand us plain decimal
                    // literals, never expression fragments.
                    open: numeric(&bar.open)?.to_string(),
                    high: numeric(&bar.high)?.to_string(),
                    low: numeric(&bar.low)?.to_string(),
                    close: numeric(&bar.close)?.to_string(),
                    volume: numeric(&bar.volume)?.to_string(),
                    trade_count: bar.trade_count,
                    revision: 0,
                    dedup_key,
                };
                insert.write(&row).await?;
            }
            insert.end().await?;
        }
        Ok(())
    }
}

fn nanos(t: DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap_or(0)
}

/// Validates that a collected value is a plain decimal literal — a
/// defense-in-depth check on the collector ingestion boundary, kept even though
/// inserts are now typed (no string interpolation).
fn numeric(s: &str) -> anyhow::Result<&str> {
    let ok = !s.is_empty()
        && s.chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_digit() || c == '.' || (i == 0 && c == '-'))
        && s.chars().filter(|&c| c == '.').count() <= 1;
    anyhow::ensure!(ok, "invalid numeric literal from collector: {s:?}");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_accepts_decimals_and_rejects_injection() {
        assert!(numeric("123.45").is_ok());
        assert!(numeric("-0.5").is_ok());
        assert!(numeric("42").is_ok());
        assert!(numeric("1.2.3").is_err());
        assert!(numeric("1e5").is_err());
        assert!(numeric("1); DROP TABLE market_bars;--").is_err());
        assert!(numeric("").is_err());
    }
}
