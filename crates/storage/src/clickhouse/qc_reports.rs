//! The `data_qc` battery's production path: read a series, grade it, persist the
//! report (Set P P-2.2; DATA-005 §7).
//!
//! `data_qc_reports` (migration 0100) had **no reader and no writer anywhere** — the
//! battery (`dataplane::qc`) ran only inside an example, and the job worker graded
//! with a separate four-line threshold function whose letters disagreed with it.
//! This module is the bridge, here for the same reason `census.rs` is: it is the
//! crate that already holds both the ClickHouse read and the Postgres write.
//!
//! Persisting is idempotent under one config version (`uq_qc_report`), so re-grading
//! an unchanged window returns the existing report id rather than a new row, and a
//! new config version is a new row — history stays answerable.

use chrono::{DateTime, Utc};
use clickhouse::Row;
use dataplane::qc::{self, QcBar, QcConfig, QcContext, QcReport, QcSubject};
use serde::Deserialize;
use uuid::Uuid;

use super::{connect, ChError};

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[derive(Row, Deserialize)]
struct Wire {
    event_ns: i64,
    knowledge_ns: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    revised: u8,
}

/// The latest-known version of each bar of one series over `[from, to)` (bar
/// opens), as the battery reads it.
///
/// # Errors
/// The backend.
pub async fn load_qc_bars(
    url: &str,
    symbol: &str,
    venue: &str,
    period_secs: u32,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<QcBar>, ChError> {
    let ns = |t: DateTime<Utc>| t.timestamp_nanos_opt().unwrap_or(i64::MAX);
    let rows: Vec<Wire> = connect(url)
        .query(&format!(
            "SELECT toInt64(toUnixTimestamp64Nano(b.event_time)) AS event_ns,
                    toInt64(toUnixTimestamp64Nano(argMax(b.knowledge_time, (b.knowledge_time, b.revision_seq)))) AS knowledge_ns,
                    toFloat64(argMax(b.open,   (b.knowledge_time, b.revision_seq))) AS open,
                    toFloat64(argMax(b.high,   (b.knowledge_time, b.revision_seq))) AS high,
                    toFloat64(argMax(b.low,    (b.knowledge_time, b.revision_seq))) AS low,
                    toFloat64(argMax(b.close,  (b.knowledge_time, b.revision_seq))) AS close,
                    toFloat64(argMax(b.volume, (b.knowledge_time, b.revision_seq))) AS volume,
                    toUInt8(max(b.revision_seq) > 0) AS revised
               FROM market_bar AS b
              WHERE b.instrument_id IN (SELECT instrument_id FROM instrument_symbol_dim WHERE symbol = '{s}')
                AND b.venue_id IN (SELECT venue_id FROM venue_dim WHERE name = '{v}')
                AND b.period_secs = {period_secs}
                AND b.event_time >= fromUnixTimestamp64Nano(toInt64({f}))
                AND b.event_time <  fromUnixTimestamp64Nano(toInt64({t}))
              GROUP BY b.event_time
              ORDER BY b.event_time",
            s = esc(symbol),
            v = esc(venue),
            f = ns(from),
            t = ns(to),
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|r| QcBar {
            event_ns: r.event_ns,
            knowledge_ns: r.knowledge_ns,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
            volume: r.volume,
            revised: r.revised != 0,
        })
        .collect())
}

/// Persist a report. Returns its id — the existing one when this exact window was
/// already graded under this config version.
///
/// # Errors
/// The database, including 0100's CHECKs (eight checks, non-empty reasons).
pub async fn persist(pg: &sqlx::PgPool, report: &QcReport) -> Result<Uuid, ChError> {
    let s = &report.subject;
    let g = &report.graded;
    let from = DateTime::from_timestamp_nanos(s.from_ns);
    let to = DateTime::from_timestamp_nanos(s.to_ns);
    let reasons = serde_json::to_value(&g.reasons).map_err(|e| ChError::Decode(e.to_string()))?;
    let checks = serde_json::to_value(&g.checks).map_err(|e| ChError::Decode(e.to_string()))?;
    // DO NOTHING, then read the existing id: the table refuses UPDATE (a report is
    // an observation), so an upsert that touches the row would be refused too.
    let inserted: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO data_qc_reports
           (instrument_id, venue_id, timeframe, window_from, window_to, grade, reasons, checks,
            config_version, digest, bars_examined, expected_bars, evidence_completeness)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
         ON CONFLICT (instrument_id, venue_id, timeframe, window_from, window_to, config_version)
         DO NOTHING
         RETURNING report_id",
    )
    .bind(&s.instrument_id)
    .bind(&s.venue_id)
    .bind(&s.timeframe)
    .bind(from)
    .bind(to)
    .bind(g.grade.as_str())
    .bind(reasons)
    .bind(checks)
    .bind(&g.config_version)
    .bind(report.digest())
    .bind(i32::try_from(report.bars_examined).unwrap_or(i32::MAX))
    .bind(i64::try_from(s.expected_bars).unwrap_or(i64::MAX))
    .bind(g.evidence_completeness())
    .fetch_optional(pg)
    .await
    .map_err(|e| ChError::Client(format!("data_qc_reports: {e}")))?;
    let id = match inserted {
        Some((id,)) => id,
        None => {
            let (id,): (Uuid,) = sqlx::query_as(
                "SELECT report_id FROM data_qc_reports
                  WHERE instrument_id = $1 AND venue_id = $2 AND timeframe = $3
                    AND window_from = $4 AND window_to = $5 AND config_version = $6",
            )
            .bind(&s.instrument_id)
            .bind(&s.venue_id)
            .bind(&s.timeframe)
            .bind(from)
            .bind(to)
            .bind(&g.config_version)
            .fetch_one(pg)
            .await
            .map_err(|e| ChError::Client(format!("data_qc_reports: {e}")))?;
            id
        }
    };
    Ok(id)
}

/// Grade one series over `[from, to)` with the battery and persist the report.
///
/// `expected_bars` is supplied (from the census), not derived from the bars'
/// own first and last timestamps: that would measure internal gaps only and
/// flatter a series whose ends are missing — the defect P-2.6's run recorded.
///
/// # Errors
/// The backend on either side.
#[allow(clippy::too_many_arguments)]
pub async fn grade_and_persist(
    pg: &sqlx::PgPool,
    url: &str,
    symbol: &str,
    venue: &str,
    timeframe: &str,
    period_secs: u32,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    expected_bars: u64,
    cfg: &QcConfig,
) -> Result<(Uuid, QcReport), ChError> {
    let bars = load_qc_bars(url, symbol, venue, period_secs, from, to).await?;
    let subject = QcSubject {
        instrument_id: symbol.to_string(),
        venue_id: venue.to_string(),
        timeframe: timeframe.to_string(),
        from_ns: from.timestamp_nanos_opt().unwrap_or(i64::MIN),
        to_ns: to.timestamp_nanos_opt().unwrap_or(i64::MAX),
        expected_bars,
        period_ns: i64::from(period_secs) * 1_000_000_000,
    };
    let report = qc::run(&bars, &subject, cfg, &QcContext::default());
    let id = persist(pg, &report).await?;
    Ok((id, report))
}

/// The newest persisted grade for an instrument and timeframe, any venue.
#[derive(Clone, Debug, PartialEq, sqlx::FromRow, serde::Serialize)]
pub struct LatestGrade {
    pub report_id: Uuid,
    pub venue_id: String,
    pub grade: String,
    pub reasons: serde_json::Value,
    pub config_version: String,
    pub graded_at: DateTime<Utc>,
}

/// The grade a manifest carries (DATA-005 §6): the report for `(instrument, timeframe)`
/// over the **deepest** series (most bars examined, then newest; ADR-P2-3a: ordering by
/// recency alone let a dead 13k-bar Kraken series graded D last decide a 1.6M-bar Coinbase
/// series' admission), or `None` when the series has never been graded —
/// which a manifest must say as "ungraded", never as a passing letter.
///
/// # Errors
/// The database.
pub async fn latest_grade(
    pg: &sqlx::PgPool,
    instrument: &str,
    timeframe: &str,
) -> Result<Option<LatestGrade>, ChError> {
    sqlx::query_as(
        "SELECT report_id, venue_id, grade, reasons, config_version, graded_at
           FROM data_qc_reports
          WHERE instrument_id = $1 AND timeframe = $2
          ORDER BY bars_examined DESC, graded_at DESC, report_id
          LIMIT 1",
    )
    .bind(instrument)
    .bind(timeframe)
    .fetch_optional(pg)
    .await
    .map_err(|e| ChError::Client(format!("data_qc_reports: {e}")))
}
