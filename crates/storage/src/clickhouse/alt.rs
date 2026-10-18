//! Alternative-data stores and their point-in-time read paths (Set P P-3.1, P-3.4,
//! P-3.5; DATA-005 §10, §12).
//!
//! Three series, one rule: **the read takes an `as_of` and nothing written later is
//! visible.** There is no function here that reads any of these tables without one.
//!
//! * `macro_observations` — per observation date, the latest *vintage* whose
//!   `knowledge_time <= as_of`. `argMax(tuple(value), …)` rather than
//!   `argMax(value, …)`: aggregate functions skip NULLs, so a vintage that
//!   reported "." would otherwise be silently replaced by an older number.
//! * `prediction_probabilities` — rows whose `knowledge_time <= as_of`; a backfilled
//!   candlestick is not readable at its own close.
//! * `text_items` — per item, the highest revision whose `available_time <= as_of`;
//!   a corrected article is two rows and the reader sees the version current then.
//!
//! Writes are plain `INSERT … VALUES` with every literal escaped, and decimals sent
//! as strings so no value ever passes through an `f64`.

use chrono::{DateTime, NaiveDate, Utc};
use clickhouse::Row;
use dataplane::source::Knowledge;
use dataplane::text::TextItem;
use dataplane::vintage::MacroObservation;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{connect, ChError};

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn ns(t: DateTime<Utc>) -> String {
    format!(
        "fromUnixTimestamp64Nano(toInt64({}))",
        t.timestamp_nanos_opt().unwrap_or(i64::MAX)
    )
}

fn dec(d: Option<Decimal>) -> String {
    d.map_or_else(
        || "NULL".to_string(),
        |v| format!("toDecimal128('{v}', 18)"),
    )
}

fn parse_dec(s: &str) -> Result<Decimal, ChError> {
    s.parse::<Decimal>()
        .map_err(|e| ChError::Decode(format!("decimal {s:?}: {e}")))
}

async fn exec(url: &str, sql: &str) -> Result<(), ChError> {
    connect(url)
        .query(sql)
        .execute()
        .await
        .map_err(|e| ChError::Insert(e.to_string()))
}

// ── macro ─────────────────────────────────────────────────────────────────────

/// Insert ALFRED vintages. Each row is validated first; an invalid one refuses the
/// whole batch rather than writing the rest around it.
///
/// # Errors
/// An invalid observation, or the backend.
pub async fn write_macro(
    url: &str,
    source_id: i32,
    rows: &[MacroObservation],
) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut values = Vec::with_capacity(rows.len());
    for r in rows {
        r.validate()
            .map_err(|e| ChError::Insert(format!("refusing {}: {e}", r.series_id)))?;
        values.push(format!(
            "('{}', toDate('{}'), toDate('{}'), {}, {}, now64(9), {}, {source_id})",
            esc(&r.series_id),
            r.observation_date,
            r.vintage_date,
            dec(r.value),
            ns(r.knowledge.time()),
            r.knowledge.quality_flags().0,
        ));
    }
    exec(
        url,
        &format!(
            "INSERT INTO macro_observations (series_id, observation_date, vintage_date, value, knowledge_time, ingest_time, quality_flags, source_id) VALUES {}",
            values.join(",")
        ),
    )
    .await?;
    Ok(rows.len())
}

/// One macro point as it stood at the read's `as_of`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct MacroPoint {
    pub observation_date: NaiveDate,
    pub vintage_date: NaiveDate,
    /// `None` is "the publisher reported missing", never zero.
    pub value: Option<Decimal>,
    pub knowledge_time: DateTime<Utc>,
    pub quality_flags: u32,
}

#[derive(Row, Deserialize)]
struct MacroWire {
    obs: String,
    vintage: String,
    x_value: Option<String>,
    kt_ns: i64,
    flags: u32,
}

/// The series over `[from, to]` (observation dates) as known at `as_of`.
///
/// # Errors
/// The backend, or a stored value that will not decode.
pub async fn macro_as_of(
    url: &str,
    series_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    as_of: DateTime<Utc>,
) -> Result<Vec<MacroPoint>, ChError> {
    let rows: Vec<MacroWire> = connect(url)
        .query(&format!(
            "SELECT toString(observation_date) AS obs,
                    toString(argMax(vintage_date, (vintage_date, ingest_time))) AS vintage,
                    argMax(tuple(toString(value)), (vintage_date, ingest_time)).1 AS x_value,
                    toInt64(toUnixTimestamp64Nano(argMax(knowledge_time, (vintage_date, ingest_time)))) AS kt_ns,
                    argMax(quality_flags, (vintage_date, ingest_time)) AS flags
               FROM macro_observations
              WHERE series_id = '{}'
                AND observation_date >= toDate('{from}') AND observation_date <= toDate('{to}')
                AND knowledge_time <= {}
              GROUP BY observation_date
              ORDER BY observation_date",
            esc(series_id),
            ns(as_of)
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    rows.into_iter()
        .map(|r| {
            Ok(MacroPoint {
                observation_date: r
                    .obs
                    .parse()
                    .map_err(|e| ChError::Decode(format!("date {}: {e}", r.obs)))?,
                vintage_date: r
                    .vintage
                    .parse()
                    .map_err(|e| ChError::Decode(format!("date {}: {e}", r.vintage)))?,
                value: r.x_value.as_deref().map(parse_dec).transpose()?,
                knowledge_time: DateTime::from_timestamp_nanos(r.kt_ns),
                quality_flags: r.flags,
            })
        })
        .collect()
}

/// Every ALFRED vintage of `series_id` known by `as_of`, **not** collapsed to the
/// latest one per observation (that is [`macro_as_of`]'s job, for a single instant).
///
/// A feature column resolves a *different* vintage at each decision instant, so the
/// reader that feeds it must hand over the whole revision history up to the end of
/// the window; the per-instant choice is then made by
/// `dataplane::vintage::series_as_of`, once, for backtest and live serve alike.
/// A vintage that reported missing is returned with `value: None`, never dropped.
///
/// # Errors
/// The backend, or a stored value that will not decode.
pub async fn macro_vintages(
    url: &str,
    series_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    as_of: DateTime<Utc>,
) -> Result<Vec<MacroObservation>, ChError> {
    let rows: Vec<MacroWire> = connect(url)
        .query(&format!(
            "SELECT toString(observation_date) AS obs,
                    toString(vintage_date) AS vintage,
                    toString(value) AS x_value,
                    toInt64(toUnixTimestamp64Nano(knowledge_time)) AS kt_ns,
                    quality_flags AS flags
               FROM macro_observations
              WHERE series_id = '{}'
                AND observation_date >= toDate('{from}') AND observation_date <= toDate('{to}')
                AND knowledge_time <= {}
              ORDER BY observation_date, vintage_date, ingest_time",
            esc(series_id),
            ns(as_of)
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    rows.into_iter()
        .map(|r| {
            let observation_date: NaiveDate = r
                .obs
                .parse()
                .map_err(|e| ChError::Decode(format!("date {}: {e}", r.obs)))?;
            let vintage_date: NaiveDate = r
                .vintage
                .parse()
                .map_err(|e| ChError::Decode(format!("date {}: {e}", r.vintage)))?;
            let at = DateTime::from_timestamp_nanos(r.kt_ns);
            // The stored flag is the provenance: a derived stamp carries the backfill
            // bit. Only the instant matters to a read; the kind is kept honest.
            let knowledge = if dataplane::QualityFlags(r.flags)
                .contains(dataplane::QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
            {
                Knowledge::Derived {
                    at,
                    from_event_time: vintage_date
                        .and_hms_opt(0, 0, 0)
                        .map_or(at, |t| DateTime::from_naive_utc_and_offset(t, Utc)),
                    declared_lag: std::time::Duration::from_secs(86_399),
                }
            } else {
                Knowledge::Observed { at }
            };
            Ok(MacroObservation {
                series_id: series_id.to_owned(),
                observation_date,
                vintage_date,
                value: r.x_value.as_deref().map(parse_dec).transpose()?,
                knowledge,
            })
        })
        .collect()
}

// ── prediction markets ───────────────────────────────────────────────────────

/// One stored probability row.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProbabilityRow {
    pub venue: String,
    pub ticker: String,
    pub event_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    pub close_probability: Decimal,
    pub volume: i64,
    pub open_interest: i64,
    pub quality_flags: u32,
    pub source_id: i32,
}

impl ProbabilityRow {
    /// Build a row from its parts, taking flags from the knowledge stamp so a
    /// derived row cannot be written unflagged.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        venue: &str,
        ticker: &str,
        event_time: DateTime<Utc>,
        knowledge: Knowledge,
        close_probability: Decimal,
        volume: i64,
        open_interest: i64,
        source_id: i32,
    ) -> Self {
        Self {
            venue: venue.into(),
            ticker: ticker.into(),
            event_time,
            knowledge_time: knowledge.time(),
            close_probability,
            volume,
            open_interest,
            quality_flags: knowledge.quality_flags().0,
            source_id,
        }
    }
}

/// Insert probability rows.
///
/// # Errors
/// A probability outside `[0, 1]`, or the backend.
pub async fn write_probabilities(url: &str, rows: &[ProbabilityRow]) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut values = Vec::with_capacity(rows.len());
    for r in rows {
        if r.close_probability < Decimal::ZERO || r.close_probability > Decimal::ONE {
            return Err(ChError::Insert(format!(
                "{} at {}: probability {} outside [0, 1]",
                r.ticker, r.event_time, r.close_probability
            )));
        }
        values.push(format!(
            "('{}', '{}', {}, {}, now64(9), {}, {}, {}, {}, {})",
            esc(&r.venue),
            esc(&r.ticker),
            ns(r.event_time),
            ns(r.knowledge_time),
            dec(Some(r.close_probability)),
            r.volume,
            r.open_interest,
            r.quality_flags,
            r.source_id,
        ));
    }
    exec(
        url,
        &format!(
            "INSERT INTO prediction_probabilities (venue, ticker, event_time, knowledge_time, ingest_time, close_probability, volume, open_interest, quality_flags, source_id) VALUES {}",
            values.join(",")
        ),
    )
    .await?;
    Ok(rows.len())
}

#[derive(Row, Deserialize)]
struct ProbWire {
    venue: String,
    ticker: String,
    et_ns: i64,
    kt_ns: i64,
    p: String,
    x_volume: i64,
    x_open_interest: i64,
    flags: u32,
    x_source_id: i32,
}

/// One market's probabilities with `event_time` in `[from, to)`, as known at
/// `as_of`: per event time, the newest row whose `knowledge_time <= as_of`.
///
/// # Errors
/// The backend, or a value that will not decode.
pub async fn probabilities_as_of(
    url: &str,
    venue: &str,
    ticker: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    as_of: DateTime<Utc>,
) -> Result<Vec<ProbabilityRow>, ChError> {
    let rows: Vec<ProbWire> = connect(url)
        .query(&format!(
            "SELECT venue, ticker,
                    toInt64(toUnixTimestamp64Nano(event_time)) AS et_ns,
                    toInt64(toUnixTimestamp64Nano(argMax(knowledge_time, (knowledge_time, ingest_time)))) AS kt_ns,
                    toString(argMax(close_probability, (knowledge_time, ingest_time))) AS p,
                    argMax(volume, (knowledge_time, ingest_time)) AS x_volume,
                    argMax(open_interest, (knowledge_time, ingest_time)) AS x_open_interest,
                    argMax(quality_flags, (knowledge_time, ingest_time)) AS flags,
                    argMax(source_id, (knowledge_time, ingest_time)) AS x_source_id
               FROM prediction_probabilities
              WHERE venue = '{}' AND ticker = '{}'
                AND event_time >= {} AND event_time < {}
                AND knowledge_time <= {}
              GROUP BY venue, ticker, event_time
              ORDER BY event_time",
            esc(venue),
            esc(ticker),
            ns(from),
            ns(to),
            ns(as_of)
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    rows.into_iter()
        .map(|r| {
            Ok(ProbabilityRow {
                venue: r.venue,
                ticker: r.ticker,
                event_time: DateTime::from_timestamp_nanos(r.et_ns),
                knowledge_time: DateTime::from_timestamp_nanos(r.kt_ns),
                close_probability: parse_dec(&r.p)?,
                volume: r.x_volume,
                open_interest: r.x_open_interest,
                quality_flags: r.flags,
                source_id: r.x_source_id,
            })
        })
        .collect()
}

// ── text ──────────────────────────────────────────────────────────────────────

/// Insert text item revisions. Each is validated; a revision is a new row.
///
/// # Errors
/// An invalid item, or the backend.
pub async fn write_text_items(url: &str, rows: &[TextItem]) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut values = Vec::with_capacity(rows.len());
    for r in rows {
        r.validate()
            .map_err(|e| ChError::Insert(format!("refusing text item {}: {e}", r.item_id)))?;
        let symbols = r
            .symbols
            .iter()
            .map(|s| format!("'{}'", esc(s)))
            .collect::<Vec<_>>()
            .join(",");
        values.push(format!(
            "('{}', {}, {}, '{}', '{}', {}, [{symbols}], {}, {}, {})",
            esc(&r.item_id),
            r.revision,
            r.source_id,
            esc(&r.external_id),
            esc(&r.headline),
            r.body
                .as_deref()
                .map_or_else(|| "NULL".to_string(), |b| format!("'{}'", esc(b))),
            ns(r.published_at),
            ns(r.ingested_time),
            ns(r.available_time),
        ));
    }
    exec(
        url,
        &format!(
            "INSERT INTO text_items (item_id, revision, source_id, external_id, headline, body, symbols, published_at, ingested_time, available_time) VALUES {}",
            values.join(",")
        ),
    )
    .await?;
    Ok(rows.len())
}

#[derive(Row, Deserialize)]
struct TextWire {
    item_id: String,
    x_revision: u32,
    x_source_id: i32,
    x_external_id: String,
    x_headline: String,
    x_body: Option<String>,
    x_symbols: Vec<String>,
    pub_ns: i64,
    ing_ns: i64,
    av_ns: i64,
}

/// Items about `symbol` published in `[from, to)`, each **as it stood at
/// `as_of`**: the highest revision whose `available_time <= as_of`. An item not
/// yet available at `as_of` does not appear at all.
///
/// # Errors
/// The backend.
pub async fn text_as_of(
    url: &str,
    symbol: Option<&str>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    as_of: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<TextItem>, ChError> {
    let sym = symbol.map_or_else(String::new, |s| format!("AND has(symbols, '{}')", esc(s)));
    let rows: Vec<TextWire> = connect(url)
        .query(&format!(
            "SELECT item_id,
                    argMax(revision, revision) AS x_revision,
                    argMax(source_id, revision) AS x_source_id,
                    argMax(external_id, revision) AS x_external_id,
                    argMax(headline, revision) AS x_headline,
                    argMax(tuple(body), revision).1 AS x_body,
                    argMax(symbols, revision) AS x_symbols,
                    toInt64(toUnixTimestamp64Nano(argMax(published_at, revision))) AS pub_ns,
                    toInt64(toUnixTimestamp64Nano(argMax(ingested_time, revision))) AS ing_ns,
                    toInt64(toUnixTimestamp64Nano(argMax(available_time, revision))) AS av_ns
               FROM text_items FINAL
              WHERE available_time <= {}
                AND published_at >= {} AND published_at < {}
                {sym}
              GROUP BY item_id
              ORDER BY pub_ns DESC, item_id
              LIMIT {limit}",
            ns(as_of),
            ns(from),
            ns(to)
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|r| TextItem {
            item_id: r.item_id,
            revision: r.x_revision,
            source_id: r.x_source_id,
            external_id: r.x_external_id,
            headline: r.x_headline,
            body: r.x_body,
            symbols: r.x_symbols,
            published_at: DateTime::from_timestamp_nanos(r.pub_ns),
            ingested_time: DateTime::from_timestamp_nanos(r.ing_ns),
            available_time: DateTime::from_timestamp_nanos(r.av_ns),
        })
        .collect())
}
