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
use dataplane::quality::QualityFlags;
use dataplane::source::Knowledge;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{ChClient, ChError};

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
    /// Which series this is. An enum, not a free-form string: `"Realized"` or a
    /// typo would both be accepted by a `String` and neither would ever be read
    /// back by [`read_funding`].
    pub kind: FundingKind,
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
                kind: r.kind.as_str().to_string(),
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

/// One series' heartbeat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeartbeatRow {
    /// `"funding"`, `"open_interest"` or `"option_chain"`.
    pub series: String,
    pub venue: String,
    pub instrument: String,
    pub last_event_time: DateTime<Utc>,
    pub rows_written: u64,
}

/// Record that forward series produced rows, so the silence alarm has one cheap
/// place to look.
///
/// Batched, because a single ticker poll covers every perpetual the venue lists —
/// 279 of them on Kraken Futures — and the alarm is per instrument. One insert per
/// instrument per hour would make the heartbeat more expensive than the
/// observations it reports on.
///
/// # Errors
/// Backend failures.
pub async fn write_heartbeats(url: &str, rows: &[HeartbeatRow]) -> Result<usize, ChError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let client = super::connect(url);
    let mut insert = client.insert("forward_series_heartbeat").map_err(|e| ChError::Insert(e.to_string()))?;
    let now = ns(Utc::now());
    for r in rows {
        insert
            .write(&WireHeartbeat {
                series: r.series.clone(),
                venue: r.venue.clone(),
                instrument: r.instrument.clone(),
                last_event_time: ns(r.last_event_time),
                last_ingest_time: now,
                rows_written: r.rows_written,
            })
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))?;
    Ok(rows.len())
}

/// The surrogate id of every declared source, keyed by name.
///
/// Read from `dataplane.source` in Postgres, which is where the ids are defined
/// (migration 0046, extended by 0098). Resolved once at startup and handed to a
/// collector's sink: a row whose source cannot be resolved must be refused, and it
/// cannot be refused by a writer that was handed an `i32` someone guessed.
///
/// # Errors
/// [`ChError::Client`] if the catalogue cannot be read.
pub async fn source_ids(pg: &sqlx::PgPool) -> Result<std::collections::HashMap<String, i32>, ChError> {
    let rows: Vec<(String, i32)> = sqlx::query_as("SELECT name, source_id FROM dataplane.source")
        .fetch_all(pg)
        .await
        .map_err(|e| ChError::Client(format!("reading dataplane.source: {e}")))?;
    Ok(rows.into_iter().collect())
}

/// Record that one forward series produced rows.
///
/// A convenience over [`write_heartbeats`] for a caller that has exactly one.
///
/// # Errors
/// Backend failures.
pub async fn beat(url: &str, series: &str, venue: &str, instrument: &str, last_event: DateTime<Utc>, rows: u64) -> Result<(), ChError> {
    write_heartbeats(
        url,
        &[HeartbeatRow {
            series: series.to_string(),
            venue: venue.to_string(),
            instrument: instrument.to_string(),
            last_event_time: last_event,
            rows_written: rows,
        }],
    )
    .await
    .map(|_| ())
}

// ── reading ───────────────────────────────────────────────────────────────────
//
// The writers above had no reader for as long as they existed, and the tables they
// target have been empty the whole time. That combination is why this section is
// shaped the way it is: everything here is built for the case where there is
// nothing to return, because that is the case that has been true on the live
// database every day since the schema landed.
//
// Three facts hide inside one empty vector, and they are not interchangeable:
//
//   1. the series exists and the window genuinely holds no observation,
//   2. the series is not in this database at all, so nothing was measured,
//   3. the query failed, so nothing is known either way.
//
// (2) is not hypothetical: a previous investigation of these very tables reported
// them missing, having queried the `default` database while the platform writes to
// `trading`. An `Option<Vec<_>>` would have answered that question, and a bare
// `Vec` did not. So the return type is `SeriesRead`, which has no `Default` and no
// accessor that hands back an empty vector for an unmeasured series, and failure
// stays in the `Err` arm where it cannot be mistaken for a measurement.
//
// The second rule this section enforces is that an absent rate is never zero.
// `FundingRate` is a newtype with no `Default`, no zero constructor, no `Sum` and
// no arithmetic: `rates.iter().sum()` and `unwrap_or_default()` do not compile, so
// the carry on a perpetual cannot quietly become flat because a settlement was
// never collected. Absence is `FundingAt::NotRecorded`, which carries no number at
// all.

/// Which funding series a read is asking for.
///
/// Required, never defaulted. A realized settlement is not knowable before its
/// instant and a prediction is knowable now; blending them in one result set is
/// how tomorrow's funding reaches today's strategy, so the caller has to say which
/// it means and there is no third answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FundingKind {
    /// Settled. Not knowable before `event_time`.
    Realized,
    /// The venue's forecast of the next settlement.
    Predicted,
}

impl FundingKind {
    /// The `kind` column's value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Realized => "realized",
            Self::Predicted => "predicted",
        }
    }

    /// Parse a stored `kind`. An unrecognised value is an error, not a default:
    /// silently bucketing an unknown kind as `realized` would do exactly the
    /// blending this enum exists to prevent.
    ///
    /// # Errors
    /// [`ChError::Decode`] if the column holds neither known value.
    pub fn from_column(s: &str) -> Result<Self, ChError> {
        match s {
            "realized" => Ok(Self::Realized),
            "predicted" => Ok(Self::Predicted),
            other => Err(ChError::Decode(format!("funding kind {other:?} is neither realized nor predicted"))),
        }
    }
}

/// A funding rate that was actually recorded.
///
/// Deliberately not `Default`, with no zero constructor, no `Sum` and no
/// arithmetic operators. Funding is money: a rate that was never collected must
/// not be able to enter an accrual as `0`, and the way to guarantee that is to
/// make the zero unreachable rather than to warn about it in a comment.
/// `unwrap_or_default()`, `sum()` and `+` on these do not compile.
///
/// The only way to obtain one is to decode a row that exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FundingRate(Decimal);

impl FundingRate {
    /// The recorded value. Naming this `get` rather than implementing `Deref` or
    /// `Into` keeps the unwrap explicit at every call site.
    #[must_use]
    pub const fn get(self) -> Decimal {
        self.0
    }

    fn decode(raw: &str) -> Result<Self, ChError> {
        raw.parse::<Decimal>().map(Self).map_err(|e| ChError::Decode(format!("funding rate {raw:?}: {e}")))
    }
}

impl std::fmt::Display for FundingRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An open-interest level that was actually recorded. Same discipline as
/// [`FundingRate`]: an uncollected hour is not an open interest of zero, which
/// would read as every position having been closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OpenInterestLevel(Decimal);

impl OpenInterestLevel {
    #[must_use]
    pub const fn get(self) -> Decimal {
        self.0
    }

    fn decode(raw: &str) -> Result<Self, ChError> {
        raw.parse::<Decimal>().map(Self).map_err(|e| ChError::Decode(format!("open interest {raw:?}: {e}")))
    }
}

fn decode_opt(raw: Option<&str>, what: &str) -> Result<Option<Decimal>, ChError> {
    match raw {
        None => Ok(None),
        Some(s) => s.parse::<Decimal>().map(Some).map_err(|e| ChError::Decode(format!("{what} {s:?}: {e}"))),
    }
}

/// The outcome of a forward-series read.
///
/// There is no `Default`, no accessor that yields an empty vector for an absent
/// series, and no `From<SeriesRead<T>> for Vec<T>`. A caller that wants the rows
/// has to say what it intends to do when the series is not there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeriesRead<T> {
    /// The table exists and the window was queried. `rows` may be empty, and an
    /// empty `rows` here is a *measurement*: nothing was recorded in that window.
    Present {
        /// Latest revision per observation known by the query's `as_of`, ordered by
        /// event time.
        rows: Vec<T>,
    },
    /// The table is not in this database. Nothing was measured, and this is not
    /// "no funding" — it is "no series". `database` is included because the last
    /// time someone asked this question they were connected to the wrong one.
    NoSuchSeries { table: &'static str, database: String },
}

impl<T> SeriesRead<T> {
    /// The rows, if the series exists at all. `None` means unmeasured.
    #[must_use]
    pub fn measured(self) -> Option<Vec<T>> {
        match self {
            Self::Present { rows } => Some(rows),
            Self::NoSuchSeries { .. } => None,
        }
    }

    /// True when the store has no such series — distinct from an empty window.
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::NoSuchSeries { .. })
    }

    /// True when the series exists and the window held nothing. Distinct from
    /// [`Self::is_absent`], and the distinction is the point.
    #[must_use]
    pub fn is_measured_empty(&self) -> bool {
        matches!(self, Self::Present { rows } if rows.is_empty())
    }
}

/// A point-in-time window over one `(venue, instrument)`.
///
/// `as_of` has no default and is not an `Option`. Every read in this module is
/// point-in-time; there is no method that omits the horizon, and adding one would
/// let a study see a settlement before the platform could have known it. A caller
/// that means "now" says so with [`SeriesQuery::as_of_now`], which is still a
/// horizon — just one it chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesQuery {
    pub venue: String,
    pub instrument: String,
    /// Inclusive lower bound on `event_time`.
    pub from: DateTime<Utc>,
    /// Exclusive upper bound on `event_time`.
    pub to: DateTime<Utc>,
    /// Rows whose `knowledge_time` is after this are invisible.
    pub as_of: DateTime<Utc>,
}

impl SeriesQuery {
    /// A window read as the world stood at `as_of`.
    #[must_use]
    pub fn new(venue: impl Into<String>, instrument: impl Into<String>, from: DateTime<Utc>, to: DateTime<Utc>, as_of: DateTime<Utc>) -> Self {
        Self { venue: venue.into(), instrument: instrument.into(), from, to, as_of }
    }

    /// A window read as the world stands now. Still a horizon: rows stamped in the
    /// future stay invisible.
    #[must_use]
    pub fn as_of_now(venue: impl Into<String>, instrument: impl Into<String>, from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self::new(venue, instrument, from, to, Utc::now())
    }
}

/// One resolved funding observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FundingPoint {
    pub venue: String,
    pub instrument: String,
    pub kind: FundingKind,
    /// The settlement instant.
    pub event_time: DateTime<Utc>,
    /// When the platform could first have known it.
    pub knowledge_time: DateTime<Utc>,
    pub rate: FundingRate,
    pub relative_rate: Option<FundingRate>,
    pub quality_flags: QualityFlags,
    pub source_id: i32,
}

impl FundingPoint {
    /// Whether Gate 15 may take forward evidence from this row (D-P3): true only
    /// for rows whose knowledge time was a real receipt.
    #[must_use]
    pub const fn is_forward_evidence(&self) -> bool {
        !self.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
    }
}

/// One resolved open-interest sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenInterestPoint {
    pub venue: String,
    pub instrument: String,
    pub event_time: DateTime<Utc>,
    pub knowledge_time: DateTime<Utc>,
    pub open_interest: OpenInterestLevel,
    pub mark_price: Option<Decimal>,
    pub quality_flags: QualityFlags,
    pub source_id: i32,
}

/// The last thing a forward series wrote, for the silence alarm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heartbeat {
    pub series: String,
    pub venue: String,
    pub instrument: String,
    pub last_event_time: DateTime<Utc>,
    pub last_ingest_time: DateTime<Utc>,
    pub rows_written: u64,
}

/// What a point lookup found. None of these variants is a number that could be
/// added to a carry by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FundingAt {
    /// A settlement was recorded and this is it.
    Settled(Box<FundingPoint>),
    /// The series exists, and no settlement was recorded in the window searched.
    /// **This is not a funding rate of zero.** A perpetual's carry over this
    /// interval is unknown, and a backtest that needs it must either skip the
    /// interval or refuse the study — it may not treat it as free.
    NotRecorded { searched_back_to: DateTime<Utc>, at: DateTime<Utc> },
    /// The funding series is not in this database at all.
    SeriesAbsent { table: &'static str, database: String },
}

const FUNDING_TABLE: &str = "funding_rates";
const OPEN_INTEREST_TABLE: &str = "open_interest";
const HEARTBEAT_TABLE: &str = "forward_series_heartbeat";

#[derive(Row, Deserialize)]
struct Probe {
    db: String,
    n: u64,
}

/// `(exists, database)` for one table, asked of the connection's *own* database.
///
/// `currentDatabase()` rather than a name passed in, and the name comes back with
/// the answer: the failure this closes is a reader that reports a series missing
/// because it was looking in `default` while the writers were filling `trading`.
async fn probe(client: &ChClient, table: &str) -> Result<(bool, String), ChError> {
    let sql = format!(
        "SELECT currentDatabase() AS db,
                countIf(name = '{table}') AS n
           FROM system.tables
          WHERE database = currentDatabase()"
    );
    let p: Probe = client.query(&sql).fetch_one().await.map_err(|e| ChError::Client(e.to_string()))?;
    Ok((p.n > 0, p.db))
}

/// The point-in-time predicate every read in this module shares.
///
/// Built once, here, so a new read cannot be written without it. `knowledge_time
/// <= as_of` is the whole of the discipline these tables are keyed for.
fn pit_window(from: DateTime<Utc>, to: DateTime<Utc>, as_of: DateTime<Utc>) -> String {
    format!(
        "event_time >= fromUnixTimestamp64Nano({f}) AND event_time < fromUnixTimestamp64Nano({t})
            AND knowledge_time <= fromUnixTimestamp64Nano({k})",
        f = ns(from),
        t = ns(to),
        k = ns(as_of),
    )
}

/// The funding read's SQL. A pure function so the point-in-time clause and the
/// revision ordering are testable without a database — which matters here, because
/// the table this reads has been empty since it was created.
fn funding_sql(q: &SeriesQuery) -> String {
    format!(
        "SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS event_ns,
                toInt64(toUnixTimestamp64Nano(knowledge_time)) AS knowledge_ns,
                toString(rate) AS rate,
                toString(relative_rate) AS relative_rate,
                quality_flags,
                source_id
           FROM {FUNDING_TABLE}
          WHERE venue = ? AND instrument = ? AND kind = ?
            AND {window}
          ORDER BY event_time, knowledge_time, ingest_time",
        window = pit_window(q.from, q.to, q.as_of),
    )
}

/// The open-interest read's SQL. Same shape, same PIT clause.
fn open_interest_sql(q: &SeriesQuery) -> String {
    format!(
        "SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS event_ns,
                toInt64(toUnixTimestamp64Nano(knowledge_time)) AS knowledge_ns,
                toString(open_interest) AS open_interest,
                toString(mark_price) AS mark_price,
                quality_flags,
                source_id
           FROM {OPEN_INTEREST_TABLE}
          WHERE venue = ? AND instrument = ?
            AND {window}
          ORDER BY event_time, knowledge_time, ingest_time",
        window = pit_window(q.from, q.to, q.as_of),
    )
}

#[derive(Row, Deserialize)]
struct WireFundingOut {
    event_ns: i64,
    knowledge_ns: i64,
    rate: String,
    relative_rate: Option<String>,
    quality_flags: u32,
    source_id: i32,
}

#[derive(Row, Deserialize)]
struct WireOpenInterestOut {
    event_ns: i64,
    knowledge_ns: i64,
    open_interest: String,
    mark_price: Option<String>,
    quality_flags: u32,
    source_id: i32,
}

#[derive(Row, Deserialize)]
struct WireHeartbeatOut {
    series: String,
    venue: String,
    instrument: String,
    last_event_ns: i64,
    last_ingest_ns: i64,
    rows_written: u64,
}

fn instant(ns_value: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_nanos(ns_value)
}

/// Keep the latest revision per `event_time`.
///
/// Rows arrive ordered by `(event_time, knowledge_time, ingest_time)` and the table
/// is a `ReplacingMergeTree` whose parts may not have merged yet, so the last row
/// for an instant is its newest version known by the horizon. The whole row is
/// kept together — resolving each column independently could assemble a settlement
/// that never existed, with one column from one revision and another from the next.
fn keep_latest_revision<T>(rows: Vec<T>, key: impl Fn(&T) -> DateTime<Utc>) -> Vec<T> {
    let mut out: Vec<T> = Vec::with_capacity(rows.len());
    for r in rows {
        if out.last().is_some_and(|p| key(p) == key(&r)) {
            out.pop();
        }
        out.push(r);
    }
    out
}

/// Read one window of funding observations, point-in-time.
///
/// An empty `Present { rows }` means the series exists and nothing was recorded —
/// which, on this platform today, is what every window returns, because the
/// collector has never been switched on. [`SeriesRead::NoSuchSeries`] means the
/// table is not there, and that is a different fact.
///
/// # Errors
/// [`ChError::Client`] if the query failed — the third fact, kept in `Err` so it
/// cannot be read as an absence of funding — or [`ChError::Decode`] if a stored
/// value would not parse. A rate that will not decode is refused rather than
/// dropped: a silently skipped settlement is a hole nobody sees.
pub async fn read_funding(url: &str, q: &SeriesQuery, kind: FundingKind) -> Result<SeriesRead<FundingPoint>, ChError> {
    let client = super::connect(url);
    let (exists, database) = probe(&client, FUNDING_TABLE).await?;
    if !exists {
        return Ok(SeriesRead::NoSuchSeries { table: FUNDING_TABLE, database });
    }
    let raw: Vec<WireFundingOut> = client
        .query(&funding_sql(q))
        .bind(q.venue.as_str())
        .bind(q.instrument.as_str())
        .bind(kind.as_str())
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;

    let mut rows = Vec::with_capacity(raw.len());
    for r in raw {
        rows.push(FundingPoint {
            venue: q.venue.clone(),
            instrument: q.instrument.clone(),
            kind,
            event_time: instant(r.event_ns),
            knowledge_time: instant(r.knowledge_ns),
            rate: FundingRate::decode(&r.rate)?,
            relative_rate: decode_opt(r.relative_rate.as_deref(), "relative funding rate")?.map(FundingRate),
            quality_flags: QualityFlags(r.quality_flags),
            source_id: r.source_id,
        });
    }
    Ok(SeriesRead::Present { rows: keep_latest_revision(rows, |p| p.event_time) })
}

/// Read one window of open-interest samples, point-in-time. Same three-way answer
/// as [`read_funding`].
///
/// # Errors
/// As [`read_funding`].
pub async fn read_open_interest(url: &str, q: &SeriesQuery) -> Result<SeriesRead<OpenInterestPoint>, ChError> {
    let client = super::connect(url);
    let (exists, database) = probe(&client, OPEN_INTEREST_TABLE).await?;
    if !exists {
        return Ok(SeriesRead::NoSuchSeries { table: OPEN_INTEREST_TABLE, database });
    }
    let raw: Vec<WireOpenInterestOut> = client
        .query(&open_interest_sql(q))
        .bind(q.venue.as_str())
        .bind(q.instrument.as_str())
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;

    let mut rows = Vec::with_capacity(raw.len());
    for r in raw {
        rows.push(OpenInterestPoint {
            venue: q.venue.clone(),
            instrument: q.instrument.clone(),
            event_time: instant(r.event_ns),
            knowledge_time: instant(r.knowledge_ns),
            open_interest: OpenInterestLevel::decode(&r.open_interest)?,
            mark_price: decode_opt(r.mark_price.as_deref(), "mark price")?,
            quality_flags: QualityFlags(r.quality_flags),
            source_id: r.source_id,
        });
    }
    Ok(SeriesRead::Present { rows: keep_latest_revision(rows, |p| p.event_time) })
}

/// Every forward series' last write.
///
/// This is operational metadata, not a measurement of the market: the table has no
/// `knowledge_time` and none is invented for it. It answers "is the collector
/// alive", and an empty `Present` is the honest answer today — nothing has ever
/// beaten.
///
/// # Errors
/// As [`read_funding`].
pub async fn read_heartbeats(url: &str) -> Result<SeriesRead<Heartbeat>, ChError> {
    let client = super::connect(url);
    let (exists, database) = probe(&client, HEARTBEAT_TABLE).await?;
    if !exists {
        return Ok(SeriesRead::NoSuchSeries { table: HEARTBEAT_TABLE, database });
    }
    let raw: Vec<WireHeartbeatOut> = client
        .query(&format!(
            "SELECT series, venue, instrument,
                    toInt64(toUnixTimestamp64Nano(last_event_time)) AS last_event_ns,
                    toInt64(toUnixTimestamp64Nano(last_ingest_time)) AS last_ingest_ns,
                    rows_written
               FROM {HEARTBEAT_TABLE} FINAL
              ORDER BY series, venue, instrument"
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(e.to_string()))?;
    Ok(SeriesRead::Present {
        rows: raw
            .into_iter()
            .map(|r| Heartbeat {
                series: r.series,
                venue: r.venue,
                instrument: r.instrument,
                last_event_time: instant(r.last_event_ns),
                last_ingest_time: instant(r.last_ingest_ns),
                rows_written: r.rows_written,
            })
            .collect(),
    })
}

/// The realized settlement in force at `at_time`: the most recent one at or before
/// it, within `lookback`, as known at `as_of`.
///
/// This is the call a funding accrual actually wants, and the reason it does not
/// return a number. The answer to "what did this perpetual cost to hold over that
/// hour" when nothing was collected is not "nothing" — it is "unknown", and
/// [`FundingAt::NotRecorded`] is the only way to say so without handing the caller
/// a zero it can add.
///
/// # Errors
/// As [`read_funding`].
pub async fn funding_at(
    url: &str,
    venue: &str,
    instrument: &str,
    at_time: DateTime<Utc>,
    lookback: chrono::Duration,
    as_of: DateTime<Utc>,
) -> Result<FundingAt, ChError> {
    let searched_back_to = at_time - lookback;
    // `to` is exclusive, so nudge past the instant itself: a settlement stamped
    // exactly at `at_time` is in force at `at_time`.
    let q = SeriesQuery::new(venue, instrument, searched_back_to, at_time + chrono::Duration::nanoseconds(1), as_of);
    match read_funding(url, &q, FundingKind::Realized).await? {
        SeriesRead::NoSuchSeries { table, database } => Ok(FundingAt::SeriesAbsent { table, database }),
        SeriesRead::Present { mut rows } => match rows.pop() {
            Some(p) => Ok(FundingAt::Settled(Box::new(p))),
            None => Ok(FundingAt::NotRecorded { searched_back_to, at: at_time }),
        },
    }
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
            kind: FundingKind::Realized,
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

    // ── reading ──────────────────────────────────────────────────────────────

    fn point(event: DateTime<Utc>, knowledge: DateTime<Utc>, rate: &str) -> FundingPoint {
        FundingPoint {
            venue: "kraken_futures".into(),
            instrument: "PF_XBTUSD".into(),
            kind: FundingKind::Realized,
            event_time: event,
            knowledge_time: knowledge,
            rate: FundingRate::decode(rate).unwrap(),
            relative_rate: None,
            quality_flags: QualityFlags::NONE,
            source_id: 17,
        }
    }

    /// The three facts an empty `Vec` would have conflated, told apart without a
    /// database. This is the test the whole read section exists for.
    #[test]
    fn an_empty_window_an_absent_series_and_a_failure_are_three_different_answers() {
        let empty: SeriesRead<FundingPoint> = SeriesRead::Present { rows: Vec::new() };
        let absent: SeriesRead<FundingPoint> = SeriesRead::NoSuchSeries { table: FUNDING_TABLE, database: "default".into() };

        assert!(empty.is_measured_empty(), "the series exists and the window held nothing");
        assert!(!empty.is_absent());
        assert!(absent.is_absent(), "the series is not in this database");
        assert!(!absent.is_measured_empty());

        // And the difference survives the accessor: measured() is the only way out,
        // and it refuses to invent rows for a series that does not exist.
        assert_eq!(empty.measured(), Some(Vec::new()));
        assert_eq!(absent.measured(), None);
        // The third fact never becomes a value at all — it stays in `Err`.
    }

    /// A backend failure must not arrive as "no funding". Nothing is mocked here:
    /// the read is pointed at a port nothing listens on, which is the cheapest
    /// honest way to produce a real failure.
    #[tokio::test]
    async fn a_failed_query_is_an_error_not_an_empty_series() {
        let q = SeriesQuery::new("kraken_futures", "PF_XBTUSD", t(0), t(6), t(6));
        let fut = read_funding("http://127.0.0.1:1/trading", &q, FundingKind::Realized);
        let r = tokio::time::timeout(std::time::Duration::from_secs(20), fut).await.expect("the read should fail fast, not hang");
        assert!(matches!(r, Err(ChError::Client(_))), "a refused connection must surface, got {r:?}");
    }

    /// Every read in this module is point-in-time. The clause is built in one
    /// place so a new read cannot be written without it, and this asserts that
    /// place is actually used by each of them.
    #[test]
    fn every_read_filters_on_knowledge_time() {
        let q = SeriesQuery::new("kraken_futures", "PF_XBTUSD", t(0), t(6), t(5));
        for sql in [funding_sql(&q), open_interest_sql(&q)] {
            assert!(sql.contains("knowledge_time <= fromUnixTimestamp64Nano"), "a read without a horizon can see the future:\n{sql}");
            assert!(sql.contains(&ns(t(5)).to_string()), "the horizon in the SQL must be the caller's as_of");
        }
    }

    /// The window is half-open on event time, and the horizon is a separate axis
    /// from it. Getting these confused is how a study reads a settlement it could
    /// not have known.
    #[test]
    fn the_window_is_half_open_and_the_horizon_is_a_separate_axis() {
        let q = SeriesQuery::new("kraken_futures", "PF_XBTUSD", t(1), t(6), t(9));
        let sql = funding_sql(&q);
        assert!(sql.contains(&format!("event_time >= fromUnixTimestamp64Nano({})", ns(t(1)))));
        assert!(sql.contains(&format!("event_time < fromUnixTimestamp64Nano({})", ns(t(6)))));
        assert!(sql.contains(&format!("knowledge_time <= fromUnixTimestamp64Nano({})", ns(t(9)))));
    }

    /// `funding_rates` and `open_interest` may be named in exactly one read each.
    /// A second `FROM` on either is a read that did not go through `pit_window`.
    #[test]
    fn each_forward_table_is_read_in_exactly_one_place() {
        let src = include_str!("forward.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        let code: String = production.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        for table in ["FROM {FUNDING_TABLE}", "FROM {OPEN_INTEREST_TABLE}"] {
            assert_eq!(code.matches(table).count(), 1, "{table} is read in more than one place; only the PIT builder may name it");
        }
    }

    /// A settlement restated after the fact resolves to the latest revision the
    /// horizon can see, and the row stays whole: one revision's rate never gets
    /// paired with another revision's flags.
    #[test]
    fn the_latest_revision_known_by_the_horizon_wins_and_the_row_stays_whole() {
        let rows = vec![
            point(t(5), t(5), "0.10"),
            point(t(5), t(6), "0.11"), // a restatement of the same settlement
            point(t(6), t(6), "0.20"),
        ];
        let resolved = keep_latest_revision(rows, |p| p.event_time);
        assert_eq!(resolved.len(), 2, "one row per settlement");
        assert_eq!(resolved[0].rate, FundingRate::decode("0.11").unwrap());
        assert_eq!(resolved[0].knowledge_time, t(6), "the rate and its knowledge time come from the same revision");
        assert_eq!(resolved[1].rate, FundingRate::decode("0.20").unwrap());
    }

    /// An hour nobody collected is not an hour that cost nothing. There is no
    /// variant of [`FundingAt`] that hands back a number without a recorded row,
    /// so a caller cannot add an absence into a carry.
    #[test]
    fn an_uncollected_hour_is_not_a_rate_of_zero() {
        let miss = FundingAt::NotRecorded { searched_back_to: t(4), at: t(5) };
        let rate = match &miss {
            FundingAt::Settled(p) => Some(p.rate),
            FundingAt::NotRecorded { .. } | FundingAt::SeriesAbsent { .. } => None,
        };
        assert!(rate.is_none(), "absence must not yield a rate");

        // Not vacuous: a recorded settlement does yield one.
        let hit = FundingAt::Settled(Box::new(point(t(5), t(5), "0.0001")));
        let rate = match &hit {
            FundingAt::Settled(p) => Some(p.rate),
            FundingAt::NotRecorded { .. } | FundingAt::SeriesAbsent { .. } => None,
        };
        assert_eq!(rate.map(FundingRate::get), Some(Decimal::from_str("0.0001").unwrap()));
    }

    /// The enforcement behind the previous test is the type, not a comment: a
    /// rate has no `Default`, no zero, and no `Sum`, so `unwrap_or_default()` and
    /// `.sum()` do not compile. This test fails the build if any of that is
    /// reintroduced.
    #[test]
    fn a_rate_has_no_zero_no_default_and_no_sum() {
        let src = include_str!("forward.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        let code: String = production.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");

        for forbidden in [
            "impl Default for FundingRate",
            "impl Default for OpenInterestLevel",
            "Default for SeriesRead",
            "Default for FundingAt",
            "Default for SeriesQuery",
            "Sum for FundingRate",
            "Sum<FundingRate>",
            "Decimal::ZERO",
            "unwrap_or_default",
        ] {
            assert!(!code.contains(forbidden), "an absent measurement must never become a zero; found `{forbidden}`");
        }

        // And no `Default` slipped in through a derive on any of them.
        for ty in ["pub struct FundingRate", "pub struct OpenInterestLevel", "pub struct SeriesQuery", "pub enum SeriesRead", "pub enum FundingAt"] {
            let at = code.find(ty).unwrap_or_else(|| panic!("{ty} not found"));
            let head = &code[..at];
            let derive_at = head.rfind("#[derive(").unwrap_or_else(|| panic!("{ty} has no derive"));
            let derive = &head[derive_at..];
            assert!(!derive.contains("Default"), "{ty} must not derive Default");
        }
    }

    /// The rate that survives the write must survive the read: the same 18-dp
    /// value, through a string, with its last digits intact. A float here would
    /// lose them in the other direction.
    #[test]
    fn a_rate_round_trips_through_the_column_exactly() {
        let original = Decimal::from_str("0.000006707804166667").unwrap();
        let m = mantissa(original).unwrap();
        // What ClickHouse would hand back for that mantissa at scale 18.
        let as_column = Decimal::from_i128_with_scale(m, SCALE).to_string();
        let read_back = FundingRate::decode(&as_column).unwrap();
        assert_eq!(read_back.get(), original);
    }

    /// A value that will not parse is refused, not skipped and not zeroed. A
    /// silently dropped settlement is a hole nobody can see afterwards.
    #[test]
    fn an_undecodable_rate_is_an_error_rather_than_a_zero() {
        for bad in ["", "n/a", "NaN", "1.2.3"] {
            let r = FundingRate::decode(bad);
            assert!(matches!(r, Err(ChError::Decode(_))), "{bad:?} must not decode");
        }
        assert!(matches!(decode_opt(Some("oops"), "mark price"), Err(ChError::Decode(_))));
        // A genuinely absent nullable column stays absent.
        assert_eq!(decode_opt(None, "mark price").unwrap(), None);
    }

    /// Realized and predicted are different knowability, so they are different
    /// reads. An unrecognised `kind` is an error rather than a silent bucketing
    /// into `realized`, which would hand a strategy the next settlement early.
    #[test]
    fn a_kind_is_named_never_defaulted() {
        assert_eq!(FundingKind::from_column("realized").unwrap(), FundingKind::Realized);
        assert_eq!(FundingKind::from_column("predicted").unwrap(), FundingKind::Predicted);
        assert!(matches!(FundingKind::from_column("REALIZED"), Err(ChError::Decode(_))));
        assert!(matches!(FundingKind::from_column(""), Err(ChError::Decode(_))));
        assert_ne!(FundingKind::Realized.as_str(), FundingKind::Predicted.as_str());
    }

    /// A row read back reports whether Gate 15 may take forward evidence from it
    /// (D-P3), from the flag the writer computed rather than from a guess.
    #[test]
    fn a_read_row_still_knows_whether_it_is_forward_evidence() {
        let observed = point(t(5), t(5), "0.1");
        assert!(observed.is_forward_evidence());

        let mut derived = point(t(5), t(6), "0.1");
        derived.quality_flags = QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        assert!(!derived.is_forward_evidence(), "a sentinel knowledge time is not a receipt");
    }

    /// Against the live store. Gated, because it needs one.
    ///
    /// Asserts the distinction that was actually got wrong on this platform: the
    /// tables are present in `trading` and absent from `default`, and a reader
    /// pointed at the wrong database must say "no such series" rather than "no
    /// funding". Run with:
    ///
    /// ```text
    /// STORAGE_FORWARD_CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
    ///   cargo test -p storage -j 2 -- --nocapture forward
    /// ```
    #[tokio::test]
    async fn against_the_live_store_the_right_database_is_empty_and_the_wrong_one_is_absent() {
        let Ok(url) = std::env::var("STORAGE_FORWARD_CLICKHOUSE_URL") else {
            eprintln!("STORAGE_FORWARD_CLICKHOUSE_URL unset — skipping");
            return;
        };
        let q = SeriesQuery::new("kraken_futures", "PF_XBTUSD", t(0), t(23), t(23));

        let here = read_funding(&url, &q, FundingKind::Realized).await.expect("the configured database must answer");
        assert!(!here.is_absent(), "funding_rates must exist in the database the writers target");

        // The same query against a database that exists but holds no such table.
        let wrong = url.rsplit_once('/').map(|(head, _)| format!("{head}/default")).expect("url must carry a database");
        let there = read_funding(&wrong, &q, FundingKind::Realized).await.expect("the wrong database must still answer");
        match there {
            SeriesRead::NoSuchSeries { table, database } => {
                assert_eq!(table, FUNDING_TABLE);
                assert_eq!(database, "default", "the reader reports which database it looked in");
            }
            SeriesRead::Present { rows } => panic!("`default` should not hold funding_rates; got {} rows", rows.len()),
        }

        // And the heartbeat read answers the operational question separately.
        let beats = read_heartbeats(&url).await.expect("heartbeats must answer");
        assert!(!beats.is_absent(), "forward_series_heartbeat must exist");
    }
}
