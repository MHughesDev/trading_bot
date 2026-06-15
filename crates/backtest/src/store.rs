//! `ClickHouse` access for the backtesting system.
//!
//! Reads and writes the canonical `market_bars` table (see
//! `clickhouse/02_bars.sql`).  Reads deduplicate revisions with `argMax` so
//! late-data corrections win, matching the `ReplacingMergeTree` semantics.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use clickhouse::Row;
use domain::payloads::bar::Timeframe;
use rust_decimal::prelude::ToPrimitive;
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

#[derive(Row, Deserialize)]
struct FeatureRowOut {
    ts_ns: i64,
    feature_name: String,
    value: String,
}

/// Versioned feature values recorded by the live feature engine, keyed by
/// `available_time` (Unix nanoseconds) then feature name.
///
/// Replaying these instead of recomputing preserves the platform's
/// versioned-feature invariant and keeps live/replay parity exact (ADR-0008):
/// the simulation sees the very values the live pipeline produced.
pub type StoredFeatures = HashMap<i64, HashMap<String, f64>>;

/// Scale of the `market_bars` OHLCV columns: `Decimal128(10)` (`Decimal(38,10)`).
const BAR_DECIMAL_SCALE: u32 = 10;

/// Row written to `market_bars` (canonical envelope schema, `clickhouse/02_bars.sql`).
///
/// Columns are serialized through the `clickhouse` crate's typed `RowBinary`
/// path — no SQL string is ever built by hand.  The `Decimal128(10)` columns
/// are sent as their unscaled `i128` mantissa (value × 10^10), which is exactly
/// the `RowBinary` wire representation of `Decimal128`; the two
/// `DateTime64(9, 'UTC')` columns are sent as their `i64` nanosecond tick value,
/// which is likewise the `RowBinary` form of `DateTime64(9)`.
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
    /// Decimal128(10) — unscaled i128 mantissa (value × 10^10).
    open: i128,
    high: i128,
    low: i128,
    close: i128,
    volume: i128,
    trade_count: u64,
    revision: u32,
    dedup_key: String,
}

impl BarStore {
    /// Connect to ClickHouse using a full URL that may include credentials and
    /// a database path (e.g. `http://user:pass@host:8123/dbname`).
    ///
    /// The clickhouse crate does not parse user/password/database from the URL
    /// itself, so they are extracted and set via dedicated builder methods.
    pub fn connect(url: &str) -> Self {
        let mut client = clickhouse::Client::default();

        // Parse out scheme + host[:port], stripping the path component that
        // ClickHouse's HTTP interface does not accept as a query route.
        let after_scheme = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .unwrap_or(url);

        let scheme = if url.starts_with("https") {
            "https"
        } else {
            "http"
        };

        // Split "user:pass@host:port/db" → (credentials, host_and_path)
        let (creds, host_path) = if let Some(at) = after_scheme.rfind('@') {
            (Some(&after_scheme[..at]), &after_scheme[at + 1..])
        } else {
            (None, after_scheme)
        };

        // Split host_path into host[:port] and optional /database
        let (host_port, db) = if let Some(slash) = host_path.find('/') {
            (&host_path[..slash], Some(&host_path[slash + 1..]))
        } else {
            (host_path, None)
        };

        client = client.with_url(format!("{}://{}", scheme, host_port));

        if let Some(cred_str) = creds {
            if let Some((user, pass)) = cred_str.split_once(':') {
                client = client.with_user(user).with_password(pass);
            } else {
                client = client.with_user(cred_str);
            }
        }

        if let Some(database) = db.filter(|d| !d.is_empty()) {
            client = client.with_database(database);
        }

        Self { client }
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

    /// Loads bars rolled up from a finer stored timeframe into `bucket_seconds`
    /// candles (e.g. stored `1m` → 5m/15m/30m for the chart).
    ///
    /// Each stored bar is first deduplicated per `available_time` (latest
    /// revision wins), then grouped into fixed `bucket_seconds` windows: open is
    /// the first bar's open, close the last bar's close, high/low the extrema,
    /// and volume/trade_count the sums.  `ts_ns` is the bucket start.
    pub async fn load_bars_bucketed(
        &self,
        instrument_id: &str,
        base_timeframe: Timeframe,
        bucket_seconds: u32,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<LoadedBar>> {
        let rows: Vec<BarRowOut> = self
            .client
            .query(
                "SELECT toInt64(toUnixTimestamp(bucket)) * 1000000000 AS ts_ns, \
                        toString(argMin(o, at)) AS open, \
                        toString(max(h)) AS high, \
                        toString(min(l)) AS low, \
                        toString(argMax(c, at)) AS close, \
                        toString(sum(v)) AS volume, \
                        sum(tc) AS trade_count \
                 FROM ( \
                     SELECT available_time AS at, \
                            toStartOfInterval(available_time, toIntervalSecond(?)) AS bucket, \
                            argMax(open, revision) AS o, \
                            argMax(high, revision) AS h, \
                            argMax(low, revision) AS l, \
                            argMax(close, revision) AS c, \
                            argMax(volume, revision) AS v, \
                            argMax(trade_count, revision) AS tc \
                     FROM market_bars \
                     WHERE instrument_id = ? AND timeframe = ? \
                       AND available_time >= fromUnixTimestamp64Nano(?) \
                       AND available_time < fromUnixTimestamp64Nano(?) \
                     GROUP BY available_time \
                 ) \
                 GROUP BY bucket ORDER BY ts_ns",
            )
            .bind(bucket_seconds)
            .bind(instrument_id)
            .bind(base_timeframe.key())
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

    /// Loads stored feature values for an instrument over `[from, to)`.
    ///
    /// Reads `features_technical` (see `clickhouse/03_features.sql`) and
    /// deduplicates revisions with `argMax(value, ingested_time)` so the
    /// latest-ingested value wins, matching the `ReplacingMergeTree` intent.
    /// The result is keyed by `available_time` (nanoseconds) then feature name,
    /// ready for the simulation to replay in place of recomputed indicators.
    ///
    /// Values are `Decimal128(18)` on the wire; they are read as strings and
    /// parsed to `f64` to match the `features` crate's indicator type (feature
    /// values are explicitly float — see `features::FeatureValue`).
    pub async fn load_features(
        &self,
        instrument_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<StoredFeatures> {
        let rows: Vec<FeatureRowOut> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(available_time) AS ts_ns, \
                        feature_name, \
                        argMax(toString(value), ingested_time) AS value \
                 FROM features_technical \
                 WHERE instrument_id = ? \
                   AND available_time >= fromUnixTimestamp64Nano(?) \
                   AND available_time < fromUnixTimestamp64Nano(?) \
                 GROUP BY available_time, feature_name \
                 ORDER BY ts_ns",
            )
            .bind(instrument_id)
            .bind(nanos(from))
            .bind(nanos(to))
            .fetch_all()
            .await?;

        let mut out: StoredFeatures = HashMap::new();
        for row in rows {
            let value: f64 = row
                .value
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid feature value '{}': {e}", row.value))?;
            out.entry(row.ts_ns)
                .or_default()
                .insert(row.feature_name, value);
        }
        Ok(out)
    }

    /// Returns the `available_time` of the most recent stored bar for the given
    /// instrument and timeframe, or `None` if no bars exist yet.
    pub async fn last_bar_time(
        &self,
        instrument_id: &str,
        timeframe: Timeframe,
    ) -> anyhow::Result<Option<DateTime<Utc>>> {
        #[derive(Row, Deserialize)]
        struct MaxTs {
            ts_ns: i64,
        }

        let rows: Vec<MaxTs> = self
            .client
            .query(
                "SELECT toUnixTimestamp64Nano(max(available_time)) AS ts_ns \
                 FROM market_bars \
                 WHERE instrument_id = ? AND timeframe = ?",
            )
            .bind(instrument_id)
            .bind(timeframe.key())
            .fetch_all()
            .await?;

        let ts_ns = rows.into_iter().next().map(|r| r.ts_ns).unwrap_or(0);
        if ts_ns == 0 {
            return Ok(None);
        }
        let secs = ts_ns / 1_000_000_000;
        let nanos_rem = (ts_ns % 1_000_000_000) as u32;
        Ok(DateTime::from_timestamp(secs, nanos_rem))
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
                    // literals, never expression fragments.  Each is then
                    // converted to the unscaled i128 the Decimal128(10) column
                    // expects on the RowBinary wire.
                    open: scaled_decimal(numeric(&bar.open)?)?,
                    high: scaled_decimal(numeric(&bar.high)?)?,
                    low: scaled_decimal(numeric(&bar.low)?)?,
                    close: scaled_decimal(numeric(&bar.close)?)?,
                    volume: scaled_decimal(numeric(&bar.volume)?)?,
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

/// Converts a plain decimal literal to the unscaled `i128` mantissa a
/// `Decimal128(10)` ClickHouse column expects on the `RowBinary` wire
/// (value × 10^10, rounded half-up to 10 decimal places).
fn scaled_decimal(s: &str) -> anyhow::Result<i128> {
    let d: Decimal = s
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid decimal literal {s:?}: {e}"))?;
    let factor = Decimal::from(10_u64.pow(BAR_DECIMAL_SCALE));
    (d * factor)
        .round()
        .to_i128()
        .ok_or_else(|| anyhow::anyhow!("decimal {s:?} out of Decimal128 range"))
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
    fn scaled_decimal_matches_decimal128_scale() {
        // value × 10^10
        assert_eq!(scaled_decimal("1").unwrap(), 10_000_000_000);
        assert_eq!(scaled_decimal("64449.5").unwrap(), 644_495_000_000_000);
        assert_eq!(scaled_decimal("0.0000000001").unwrap(), 1);
        assert_eq!(scaled_decimal("-2.5").unwrap(), -25_000_000_000);
        // Beyond 10 dp rounds to nearest at the column's scale.
        assert_eq!(scaled_decimal("0.00000000004").unwrap(), 0);
        assert_eq!(scaled_decimal("0.00000000006").unwrap(), 1);
    }

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
