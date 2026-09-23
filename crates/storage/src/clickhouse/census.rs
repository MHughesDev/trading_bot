//! Taking a silence census: the bridge between the stores and the alarm.
//!
//! [`observability::silence`] deliberately holds no database client of any kind.
//! A component that measures staleness *from* the store it is measuring cannot
//! tell a dead feed from its own outage: it either reports every series silent on
//! its own blip, or goes quiet in exactly the failure it exists to catch. So the
//! judging lives there and the reading lives here, in the crate that already
//! holds both connections.
//!
//! # What this module reads, and what it refuses to conclude
//!
//! Two stores answer two different questions and they are not interchangeable:
//!
//! * **`dataplane.forward_series`** (Postgres) is the only place the platform has
//!   written down a promise: *we undertook to collect this series every
//!   `cadence_secs`*. Expectations come from there and nowhere else. A series with
//!   no row is a series nobody promised, and it is judged
//!   [`Verdict::CannotJudge`] rather than against a threshold invented here.
//! * **ClickHouse** holds the evidence: `forward_series_heartbeat` for the
//!   forward-only series, and `market_bar`'s own two timestamp columns for the
//!   canonical bar store.
//!
//! # The canonical bar store has no second clock, and that is the finding
//!
//! `forward_series_heartbeat` carries a write clock that no market can stop: the
//! collector beats on every poll, with `rows_written = 0` when the venue had
//! nothing. `market_bar` has no such beat. Its `ingest_time` only advances when a
//! bar exists, and a bar only exists when the market traded — so for a canonical
//! feed the two clocks are *not independent*, and a per-feed write-deadline alarm
//! over it would fire every weekend for any session-bound venue. That is the alarm
//! that gets muted, and a muted alarm is worse than none.
//!
//! So this module makes exactly one claim about the canonical store, and it is a
//! claim about **us**, not about any venue: [`WHOLE_STORE`] — *every venue we
//! collect from stopped writing at the same time*. A single venue's closure cannot
//! stop the others, so that needs no calendar. It is guarded by
//! [`MIN_VENUES_FOR_WHOLE_STORE_CLAIM`], because with only one venue in the table
//! the claim silently degenerates into "that venue is shut", which is the calendar
//! this module refuses to invent. The one case that needs no guard at all is an
//! **empty** canonical store: no market closure has ever emptied a table, so that
//! is [`Verdict::NeverSeen`] and it alarms.
//!
//! Per-feed canonical verdicts stay [`Verdict::CannotJudge`] until the registry
//! carries a row for them **and** the bar persister beats — pinned by
//! `a_canonical_feed_becomes_judgeable_only_with_a_registry_row_and_a_beat`.
//!
//! [`Verdict::NeverSeen`]: observability::silence::Verdict::NeverSeen
//!
//! # The caller
//!
//! `apps/platform/src/health_jobs.rs`'s hourly tick calls [`silence_report`] from
//! its own `silence_pass`, not appended inside the tenant pass: that pass returns
//! early when Postgres will not list tenants, and appending there would let a
//! Postgres hiccup silence the silence alarm — the one failure this mechanism
//! exists to make impossible. `silence_pass` runs unconditionally after it.
//!
//! [`Verdict::CannotJudge`]: observability::silence::Verdict::CannotJudge

use std::collections::HashSet;
use std::num::NonZeroU32;

use chrono::{DateTime, Utc};
use clickhouse::Row;
use observability::silence::{
    evaluate, ArrivalDecl, Census, FeedExpectation, FeedKey, Heartbeat, RegistryRow, Roster, SilenceReport,
    WHOLE_SERIES,
};
use serde::Deserialize;
use sqlx::PgPool;

use super::forward::{self, SeriesRead};
use super::{ChClient, ChError};

/// The canonical bar store.
pub const CANONICAL_BAR_TABLE: &str = "market_bar";

/// The registry: the only place the platform records a promise to collect.
pub const REGISTRY_TABLE: &str = "dataplane.forward_series";

/// The series id under which the canonical store is judged as a whole.
///
/// Not a per-feed id: see the module docs. `market_bar` supports one defensible
/// claim and this is it.
pub const WHOLE_STORE: &str = "canonical.market_bar";

/// Below this many distinct venues in the canonical store, "every venue we
/// collect from stopped writing" is not a statement about us — it is a statement
/// about the one venue, which is a trading calendar, which this platform does not
/// have. The claim is withheld rather than guessed.
pub const MIN_VENUES_FOR_WHOLE_STORE_CLAIM: usize = 2;

/// The tolerance `dataplane.forward_series` declares for itself: the column
/// default of `alarm_after_cadences`, checked `>= 1` by the registry's own
/// constraint. Quoted here so a caller passes the platform's number rather than
/// one this module made up, and it is still passed explicitly — there is no
/// defaulted overload.
pub const REGISTRY_DEFAULT_TOLERANCE_CADENCES: u32 = 3;

// ── The registry: the only place a promise is written down ───────────────────

/// One `dataplane.forward_series` row as the Postgres driver hands it over.
/// Columns in the order the query names them.
type RegistryTuple = (String, String, String, i32, i32, i64, String, Option<i32>, Option<i32>, bool);

/// Every registered forward series, as plain rows.
///
/// # Errors
/// [`ChError::Client`] if the registry cannot be read. Never an empty `Ok`: a
/// registry that cannot be read is not a platform with no series.
pub async fn registry(pg: &PgPool) -> Result<Vec<RegistryRow>, ChError> {
    let rows: Vec<RegistryTuple> = sqlx::query_as(
        "SELECT series_id, store_table, venue, cadence_secs, alarm_after_cadences,
                disk_budget_bytes, retention_policy, downsample_after_days,
                downsample_to_secs, enabled
           FROM dataplane.forward_series
          ORDER BY series_id",
    )
    .fetch_all(pg)
    .await
    .map_err(|e| ChError::Client(format!("reading dataplane.forward_series: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|r| RegistryRow {
            series_id: r.0,
            store_table: r.1,
            venue: r.2,
            cadence_secs: r.3,
            alarm_after_cadences: r.4,
            disk_budget_bytes: r.5,
            retention_policy: r.6,
            downsample_after_days: r.7,
            downsample_to_secs: r.8,
            enabled: r.9,
            // `dataplane.forward_series` has no arrival column. `None` is the
            // honest mapping and `FeedExpectation::from_registry` turns it into a
            // claim about our own cadence rather than one about the venue.
            arrival: arrival_column(),
        })
        .collect())
}

/// The registry's arrival declaration, which does not exist yet.
///
/// A function rather than a literal `None` at the call site so that adding the
/// column is one edit in one place, and so the test below can state in code that
/// today there is nothing to read.
const fn arrival_column() -> Option<ArrivalDecl> {
    None
}

/// Expectations for every series the registry knows about.
///
/// [`Roster::WhateverAppears`] for all of them: `dataplane.forward_series` has no
/// instrument column, so the platform cannot name an instrument that never
/// appeared. That is a stated limitation, not a silent one — the series as a whole
/// is still judged, so a series that has produced nothing at all is still caught.
///
/// # Errors
/// [`ChError::Client`] if the registry cannot be read, or [`ChError::Decode`] if a
/// row violates the registry's own CHECK constraints — which is a corrupt
/// registry, not an absence of series, and must not read as one.
pub async fn forward_expectations(pg: &PgPool) -> Result<Vec<FeedExpectation>, ChError> {
    registry(pg)
        .await?
        .iter()
        .map(|r| {
            FeedExpectation::from_registry(r, Roster::WhateverAppears)
                .map_err(|e| ChError::Decode(format!("dataplane.forward_series: {e}")))
        })
        .collect()
}

// ── The forward census ───────────────────────────────────────────────────────

/// A census of the forward-only series, from `forward_series_heartbeat`.
///
/// The three-way [`SeriesRead`] collapses into the two-way [`Census`] by one rule,
/// and the rule is the point of both types:
///
/// * `Present { rows }` — the table exists and this is what it held. An empty
///   `rows` is a **measurement**, and it produces [`Verdict::NeverSeen`] for every
///   promised series, which alarms. That is the live state today.
/// * `NoSuchSeries` — the heartbeat table is not in this database, so nobody
///   looked. That is [`Census::Failed`], never `Taken { beats: [] }`: blaming a
///   collector for our own inability to find the table would be an accusation we
///   have no evidence for, and it is exactly the mistake that reported these
///   tables missing once already, from the wrong database.
/// * `Err` — the query failed. Also [`Census::Failed`].
///
/// [`Verdict::NeverSeen`]: observability::silence::Verdict::NeverSeen
pub async fn forward_census(url: &str, at: DateTime<Utc>) -> Census {
    let read = match forward::read_heartbeats(url).await {
        Ok(SeriesRead::Present { rows }) => Ok(rows.into_iter().map(beat_of).collect()),
        Ok(SeriesRead::NoSuchSeries { table, database }) => Err(format!(
            "{table} is not in database {database:?}; the collector has not been accused of anything, \
             because nobody managed to look"
        )),
        Err(e) => Err(e.to_string()),
    };
    Census::from_read(at, read)
}

fn beat_of(h: forward::Heartbeat) -> Heartbeat {
    Heartbeat {
        feed: FeedKey::new(h.series, h.venue, h.instrument),
        last_event_time: h.last_event_time,
        last_write_time: h.last_ingest_time,
        rows_written: h.rows_written,
    }
}

// ── The canonical bar census ─────────────────────────────────────────────────

/// One `(instrument, venue, period)` feed in the canonical store, with both of its
/// clocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarFeed {
    pub venue: String,
    pub instrument: String,
    pub period_secs: u32,
    /// Newest `event_time`: the venue's clock.
    pub last_event_time: DateTime<Utc>,
    /// Newest `ingest_time`: when we last wrote. **Not** an independent write
    /// clock — see the module docs — because a bar only exists if the market
    /// traded.
    pub last_write_time: DateTime<Utc>,
    /// Bars carrying that newest `ingest_time`.
    pub rows_in_last_write: u64,
}

/// What a census of the canonical store found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalBars {
    Taken { at: DateTime<Utc>, feeds: Vec<BarFeed> },
    Failed { attempted_at: DateTime<Utc>, reason: String },
}

#[derive(Row, Deserialize)]
struct WireBarFeed {
    instrument_id: i64,
    venue_id: i32,
    period_secs: u32,
    last_event_ns: i64,
    last_write_ns: i64,
    rows_in_last_write: u64,
    venue_name: String,
    symbol: String,
}

/// The canonical census SQL.
///
/// A pure function so the two properties that matter are testable without a
/// database: the point-in-time horizon, and the joins to the dimension tables
/// being `LEFT`. An `INNER JOIN` here — which is what the canonical *migration*
/// next door legitimately uses — would drop any feed the dimension does not name,
/// and a dropped feed is a dead feed reported as health.
fn canonical_sql(as_of: DateTime<Utc>) -> String {
    let horizon = as_of.timestamp_nanos_opt().unwrap_or(i64::MAX);
    format!(
        "SELECT g.instrument_id AS instrument_id,
                g.venue_id AS venue_id,
                g.period_secs AS period_secs,
                g.last_event_ns AS last_event_ns,
                g.last_write_ns AS last_write_ns,
                g.rows_in_last_write AS rows_in_last_write,
                v.venue_name AS venue_name,
                s.symbol AS symbol
           FROM (
             SELECT instrument_id, venue_id, period_secs,
                    toInt64(toUnixTimestamp64Nano(max(event_time)))  AS last_event_ns,
                    toInt64(toUnixTimestamp64Nano(max(ingest_time))) AS last_write_ns,
                    toUInt64(countIf(ingest_time = newest_write))    AS rows_in_last_write
               FROM (
                 SELECT instrument_id, venue_id, period_secs, event_time, ingest_time,
                        max(ingest_time) OVER (PARTITION BY instrument_id, venue_id, period_secs)
                            AS newest_write
                   FROM {CANONICAL_BAR_TABLE}
                  WHERE knowledge_time <= fromUnixTimestamp64Nano({horizon})
               )
              GROUP BY instrument_id, venue_id, period_secs
           ) AS g
           LEFT JOIN (
             SELECT venue_id AS vid, any(name) AS venue_name FROM venue_dim FINAL GROUP BY venue_id
           ) AS v ON v.vid = g.venue_id
           LEFT JOIN (
             SELECT instrument_id AS iid, venue_id AS vid2, argMax(symbol, valid_from) AS symbol
               FROM instrument_symbol_dim FINAL GROUP BY instrument_id, venue_id
           ) AS s ON s.iid = g.instrument_id AND s.vid2 = g.venue_id
          ORDER BY g.venue_id, g.instrument_id, g.period_secs"
    )
}

/// A dimension miss is not a reason to drop a feed. An instrument the symbol table
/// does not name is still writing, or still dead; naming it by its surrogate key
/// keeps it in the report.
fn name_or_id(name: &str, prefix: &str, id: i64) -> String {
    if name.is_empty() {
        format!("{prefix}#{id}")
    } else {
        name.to_string()
    }
}

/// Census the canonical bar store's two clocks, point-in-time.
///
/// Unlike the forward series there is no heartbeat table to read, so both clocks
/// are derived from `market_bar` itself: `max(event_time)` and `max(ingest_time)`
/// per `(instrument, venue, period)`. The horizon is applied, matching every other
/// read of this table (`backtest::store`): a census taken as of a past instant must
/// not see a row the platform could not have known about.
///
/// Never returns `Err`: a failed read is [`CanonicalBars::Failed`], which still
/// produces a finding. An error swallowed by a caller's `?` is silence.
pub async fn canonical_bar_census(url: &str, at: DateTime<Utc>) -> CanonicalBars {
    let client: ChClient = super::connect(url);
    let raw: Vec<WireBarFeed> = match client.query(&canonical_sql(at)).fetch_all().await {
        Ok(r) => r,
        Err(e) => return CanonicalBars::Failed { attempted_at: at, reason: e.to_string() },
    };

    let mut seen: HashSet<(i64, i32, u32)> = HashSet::new();
    let mut feeds = Vec::with_capacity(raw.len());
    for r in raw {
        if !seen.insert((r.instrument_id, r.venue_id, r.period_secs)) {
            // A duplicated key means a dimension join fanned out. Two rows for one
            // feed let a live one mask a dead one, so this is refused rather than
            // deduplicated on a guess.
            return CanonicalBars::Failed {
                attempted_at: at,
                reason: format!(
                    "the canonical census returned {} twice at {}s; a dimension join fanned out",
                    r.instrument_id, r.period_secs
                ),
            };
        }
        feeds.push(BarFeed {
            venue: name_or_id(&r.venue_name, "venue", i64::from(r.venue_id)),
            instrument: name_or_id(&r.symbol, "instrument", r.instrument_id),
            period_secs: r.period_secs,
            last_event_time: DateTime::from_timestamp_nanos(r.last_event_ns),
            last_write_time: DateTime::from_timestamp_nanos(r.last_write_ns),
            rows_in_last_write: r.rows_in_last_write,
        });
    }
    CanonicalBars::Taken { at, feeds }
}

impl CanonicalBars {
    /// The one expectation this store supports, and the heartbeat it is judged
    /// against — `None` when there is nothing to judge it against.
    ///
    /// Three outcomes, and the middle one is the guard:
    ///
    /// * `Some((exp, None))` — the store is **empty**. That is an existence claim,
    ///   not a staleness claim: it needs no venue count and no calendar, because no
    ///   market closure has ever emptied a table. It yields
    ///   [`Verdict::NeverSeen`](observability::silence::Verdict::NeverSeen), which
    ///   alarms.
    /// * `None` — fewer than [`MIN_VENUES_FOR_WHOLE_STORE_CLAIM`] venues have
    ///   written, at which point "everything stopped" and "that one venue is shut"
    ///   are the same sentence and this module holds no calendar that could tell
    ///   them apart. The claim is withheld rather than guessed.
    /// * `Some((exp, Some(beat)))` — judgeable on our clock.
    fn whole_store(&self, tolerance: NonZeroU32) -> Option<(FeedExpectation, Option<Heartbeat>)> {
        let CanonicalBars::Taken { feeds, .. } = self else { return None };
        if feeds.is_empty() {
            return Some((whole_store_expectation(1, tolerance), None));
        }
        let venues: HashSet<&str> = feeds.iter().map(|f| f.venue.as_str()).collect();
        if venues.len() < MIN_VENUES_FOR_WHOLE_STORE_CLAIM {
            return None;
        }
        // The longest period any feed declares: the most forgiving deadline the
        // store's own contents justify.
        let period = feeds.iter().map(|f| f.period_secs).max()?;
        let exp = whole_store_expectation(period, tolerance);
        let beat = Heartbeat {
            feed: FeedKey::new(WHOLE_STORE, WHOLE_SERIES, WHOLE_SERIES),
            last_event_time: feeds.iter().map(|f| f.last_event_time).max()?,
            last_write_time: feeds.iter().map(|f| f.last_write_time).max()?,
            rows_written: rows_in_last_write_of_any_feed(feeds),
        };
        Some((exp, Some(beat)))
    }

    /// Judge the canonical store.
    ///
    /// Three kinds of finding come out, and the split is the whole design:
    ///
    /// * one whole-store verdict, which may alarm, because "every venue stopped at
    ///   once" is a claim about us;
    /// * one `CannotJudge` per feed, because no registry row promises any of them
    ///   and this module will not invent a deadline for a feed whose write clock
    ///   stops when its market does;
    /// * on a failed read, one `Unobserved` — blind, not healthy.
    #[must_use]
    pub fn report(&self, tolerance: NonZeroU32) -> SilenceReport {
        let at = match self {
            Self::Taken { at, .. } | Self::Failed { attempted_at: at, .. } => *at,
        };
        let (exps, census) = match self {
            Self::Failed { reason, .. } => (
                // One expectation so the blind pass still names the store. With a
                // failed census `evaluate` never judges, so the deadline on this
                // expectation is unreachable — pinned by a test.
                vec![whole_store_expectation(1, tolerance)],
                Census::Failed { attempted_at: at, reason: reason.clone() },
            ),
            Self::Taken { feeds, .. } => {
                let mut beats: Vec<Heartbeat> = feeds
                    .iter()
                    .map(|f| Heartbeat {
                        feed: FeedKey::new(feed_series_id(f.period_secs), &f.venue, &f.instrument),
                        last_event_time: f.last_event_time,
                        last_write_time: f.last_write_time,
                        rows_written: f.rows_in_last_write,
                    })
                    .collect();
                let mut exps = Vec::new();
                if let Some((exp, beat)) = self.whole_store(tolerance) {
                    exps.push(exp);
                    // No beat means the store is empty. The expectation is still
                    // registered, so `evaluate` reports the promised feed as
                    // `NeverSeen` rather than reporting nothing at all.
                    beats.extend(beat);
                }
                (exps, Census::Taken { at, beats })
            }
        };
        evaluate(&exps, &census)
    }
}

fn rows_in_last_write_of_any_feed(feeds: &[BarFeed]) -> u64 {
    feeds.iter().map(|f| f.rows_in_last_write).sum()
}

/// The per-feed series id. Period is part of it because a 1-minute feed and an
/// hourly feed on the same instrument are two feeds with two deadlines, and
/// folding them together would let the hourly one's freshness cover the minute
/// one's death.
fn feed_series_id(period_secs: u32) -> String {
    format!("{WHOLE_STORE}.{period_secs}s")
}

/// An expectation covering one store as a whole, under
/// `(series_id, WHOLE_SERIES, WHOLE_SERIES)`.
///
/// [`observability::silence::Arrival::Polled`] deliberately: the deadline is on
/// our clock, and the venue clock is reported and never alarmed. There is no
/// `Arrival::Always` here — that would be a claim that no venue this platform
/// collects from ever closes, which nothing in this repository supports.
fn store_expectation(series_id: &str, store_table: &str, period_secs: u32, tolerance: NonZeroU32) -> FeedExpectation {
    let period = period_secs.max(1);
    FeedExpectation {
        spec: dataplane::retention::ForwardSeriesSpec {
            series_id: series_id.to_string(),
            store_table: store_table.to_string(),
            venue: WHOLE_SERIES.to_string(),
            cadence_secs: period,
            alarm_after_cadences: tolerance.get(),
            // Inert: `Arrival::Polled` never consults `dataplane::retention`,
            // which is the only place a budget is read. Pinned by
            // `the_whole_store_claim_never_consults_retention`.
            disk_budget_bytes: 1,
            retention: dataplane::retention::RetentionPolicy::None,
            enabled: true,
        },
        arrival: observability::silence::Arrival::Polled { poll_period_secs: period },
        roster: Roster::WhateverAppears,
    }
}

fn whole_store_expectation(period_secs: u32, tolerance: NonZeroU32) -> FeedExpectation {
    store_expectation(WHOLE_STORE, CANONICAL_BAR_TABLE, period_secs, tolerance)
}

// ── The whole pass ───────────────────────────────────────────────────────────

/// One silence pass over every store this platform writes.
///
/// This is the call a scheduled job makes. It never returns `Err`: every failure
/// becomes a finding, because a pass that returns an error a caller can `?` away
/// is a pass that can go silent in exactly the failure it exists to catch.
///
/// The shared cause is recomputed over the union of both stores' findings
/// ([`SilenceReport::merged`]), so one host or one writer process taking both
/// stores down pages once rather than once per store.
pub async fn silence_report(pg: &PgPool, ch_url: &str, at: DateTime<Utc>, tolerance: NonZeroU32) -> SilenceReport {
    let forward = match forward_expectations(pg).await {
        Ok(exps) => {
            let census = forward_census(ch_url, at).await;
            evaluate(&exps, &census)
        }
        Err(e) => {
            // The registry is unreadable, so no promise can be looked up. Judging
            // the heartbeats against no expectations at all would report every
            // live feed as undeclared and produce no alarm and no warning; naming
            // the registry itself as unobserved is the truth.
            let blind = store_expectation(REGISTRY_TABLE, "forward_series_heartbeat", 1, tolerance);
            evaluate(
                std::slice::from_ref(&blind),
                &Census::Failed { attempted_at: at, reason: e.to_string() },
            )
        }
    };
    let canonical = canonical_bar_census(ch_url, at).await.report(tolerance);
    SilenceReport::merged(at, vec![forward, canonical])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use observability::silence::{Arrival, Severity, Verdict};

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 21, 0, 0).unwrap()
    }

    fn tol() -> NonZeroU32 {
        NonZeroU32::new(REGISTRY_DEFAULT_TOLERANCE_CADENCES).expect("the registry default is positive")
    }

    /// The nine feeds `trading.market_bar` actually held on 2026-09-22, with the
    /// clocks it actually reported. Every one of them is dead; the newest write is
    /// 19 hours old and the shortest period is 60 seconds.
    fn live_feeds() -> Vec<BarFeed> {
        let h = |hours: i64| at() - chrono::Duration::hours(hours);
        vec![
            BarFeed { venue: "kraken".into(), instrument: "BTC-USD".into(), period_secs: 60, last_event_time: h(19), last_write_time: h(19), rows_in_last_write: 1 },
            BarFeed { venue: "coinbase".into(), instrument: "BTC-USD".into(), period_secs: 60, last_event_time: h(24), last_write_time: h(24), rows_in_last_write: 139 },
            BarFeed { venue: "coinbase".into(), instrument: "BTC-USDT".into(), period_secs: 3600, last_event_time: h(236), last_write_time: h(236), rows_in_last_write: 2 },
            BarFeed { venue: "kraken".into(), instrument: "ETH-USD".into(), period_secs: 60, last_event_time: h(242), last_write_time: h(242), rows_in_last_write: 1 },
            BarFeed { venue: "coinbase".into(), instrument: "ETH-USD".into(), period_secs: 60, last_event_time: h(242), last_write_time: h(242), rows_in_last_write: 285 },
            // The planted fixture: an event time in 2020, a write ten days ago.
            BarFeed { venue: "synthetic".into(), instrument: "SYN-PLANTED-AR1-424242".into(), period_secs: 3600, last_event_time: Utc.with_ymd_and_hms(2020, 7, 27, 6, 0, 0).unwrap(), last_write_time: h(265), rows_in_last_write: 5000 },
            BarFeed { venue: "binance".into(), instrument: "BTC-USD".into(), period_secs: 60, last_event_time: h(2204), last_write_time: h(2204), rows_in_last_write: 60 },
            BarFeed { venue: "coinbase".into(), instrument: "BTC-USD".into(), period_secs: 3600, last_event_time: h(2735), last_write_time: h(2397), rows_in_last_write: 18 },
            BarFeed { venue: "coinbase".into(), instrument: "ETH-USD".into(), period_secs: 3600, last_event_time: h(2402), last_write_time: h(2402), rows_in_last_write: 48 },
        ]
    }

    // ── The census SQL ───────────────────────────────────────────────────────

    /// Every read of `market_bar` in this repository carries the horizon, and this
    /// one is not an exception because it is "only" operational. A census taken as
    /// of a past instant must not see a row the platform could not have known.
    #[test]
    fn the_canonical_census_is_point_in_time() {
        let sql = canonical_sql(at());
        let ns = at().timestamp_nanos_opt().unwrap();
        assert!(
            sql.contains(&format!("knowledge_time <= fromUnixTimestamp64Nano({ns})")),
            "a census without a horizon can see the future:\n{sql}"
        );
    }

    /// The joins to the dimension tables must be `LEFT`. `canonical.rs` next door
    /// uses `INNER JOIN` against these same two tables, legitimately, and copying
    /// that here would drop any feed the symbol table does not name — which is a
    /// dead feed reported as no feed, the exact shape this whole module exists to
    /// stop.
    #[test]
    fn a_feed_the_dimension_cannot_name_is_not_dropped() {
        let sql = canonical_sql(at());
        assert_eq!(sql.matches("LEFT JOIN").count(), 2, "both dimensions are joined leniently:\n{sql}");
        assert!(!sql.contains("INNER JOIN"), "an inner join here silently drops feeds:\n{sql}");

        // And the fallback naming keeps it identifiable rather than blank.
        assert_eq!(name_or_id("", "instrument", 1004), "instrument#1004");
        assert_eq!(name_or_id("", "venue", 11), "venue#11");
        assert_eq!(name_or_id("BTC-USD", "instrument", 1005), "BTC-USD");
    }

    /// The dimension subqueries aggregate to one row per key. The symbol table is
    /// slowly-changing, so a `DISTINCT` over `(instrument_id, venue_id, symbol)`
    /// would return one row per historical symbol and fan the join out.
    #[test]
    fn the_dimension_joins_cannot_fan_out() {
        let sql = canonical_sql(at());
        assert!(sql.contains("argMax(symbol, valid_from)"), "one symbol per instrument, the newest:\n{sql}");
        assert!(sql.contains("any(name) AS venue_name"));
        assert!(!sql.contains("DISTINCT"), "DISTINCT over an SCD table is not a deduplication:\n{sql}");
    }

    // ── The canonical verdicts ───────────────────────────────────────────────

    /// The live store, judged. Every venue we collect from stopped writing, which
    /// is a statement about us and needs no calendar, so it alarms — once.
    #[test]
    fn the_canonical_store_as_it_stands_today_alarms_once_for_the_whole_store() {
        let r = CanonicalBars::Taken { at: at(), feeds: live_feeds() }.report(tol());

        let alarms = r.with_severity(Severity::Alarm);
        assert_eq!(alarms.len(), 1, "one defensible claim, one alarm: {alarms:?}");
        assert_eq!(alarms[0].feed, FeedKey::new(WHOLE_STORE, WHOLE_SERIES, WHOLE_SERIES));
        match &alarms[0].verdict {
            Verdict::CollectorSilent { write_age_secs, deadline_secs } => {
                assert_eq!(*deadline_secs, 3 * 3600, "the longest period the store declares, times the tolerance");
                assert_eq!(*write_age_secs, 19 * 3600, "the newest write anywhere in the store");
            }
            other => panic!("expected a silent whole store, got {other:?}"),
        }
        assert!(!r.all_clear());
    }

    /// And every individual feed is reported rather than judged. No registry row
    /// promises any of them, and `market_bar` has no write clock that a shut
    /// market cannot stop — so a per-feed deadline would be invented, and an
    /// invented deadline is the one that gets the alarm muted.
    #[test]
    fn no_canonical_feed_is_judged_against_a_deadline_nobody_promised() {
        let r = CanonicalBars::Taken { at: at(), feeds: live_feeds() }.report(tol());
        let unjudged: Vec<_> =
            r.findings.iter().filter(|f| matches!(f.verdict, Verdict::CannotJudge { .. })).collect();
        assert_eq!(unjudged.len(), 9, "all nine feeds are reported: {:?}", r.findings);
        for f in &unjudged {
            assert_eq!(f.severity(), Severity::Info, "honest, and not a page");
            assert!(f.feed.series.starts_with(WHOLE_STORE), "each feed names the store it came from");
        }
        assert_eq!(r.findings.len(), 10, "nine feeds plus the whole-store claim");
    }

    /// The planted fixture, whose event time is in 2020, must not contribute a
    /// six-year-old page. It is judged on our clock like everything else, and its
    /// event clock only ever appears as a reported number.
    #[test]
    fn the_planted_fixture_does_not_make_the_store_look_six_years_overdue() {
        let feeds = live_feeds();
        let syn = feeds.iter().find(|f| f.venue == "synthetic").expect("the fixture");
        assert!(syn.last_event_time < syn.last_write_time - chrono::Duration::days(2000));

        let r = CanonicalBars::Taken { at: at(), feeds }.report(tol());
        let whole = r
            .findings
            .iter()
            .find(|f| f.feed.instrument == WHOLE_SERIES)
            .expect("the whole-store finding");
        match &whole.verdict {
            // 19 hours, not six years. The event clock is not what is judged.
            Verdict::CollectorSilent { write_age_secs, .. } => assert_eq!(*write_age_secs, 19 * 3600),
            other => panic!("{other:?}"),
        }
    }

    /// A store the platform *is* writing is not alarmed by this claim, so the
    /// previous test is not vacuous.
    #[test]
    fn a_store_being_written_is_not_alarmed() {
        let feeds: Vec<BarFeed> = live_feeds()
            .into_iter()
            .map(|f| BarFeed { last_write_time: at(), ..f })
            .collect();
        let r = CanonicalBars::Taken { at: at(), feeds }.report(tol());
        assert!(r.with_severity(Severity::Alarm).is_empty(), "{:?}", r.findings);
    }

    /// The guard. With one venue in the store, "everything stopped writing" and
    /// "that venue is shut" are the same sentence, and this module does not hold a
    /// calendar that could tell them apart. The claim is withheld, and the feeds
    /// are still reported.
    #[test]
    fn with_a_single_venue_the_whole_store_claim_is_withheld_rather_than_guessed() {
        let feeds: Vec<BarFeed> =
            live_feeds().into_iter().filter(|f| f.venue == "coinbase").collect();
        assert!(feeds.len() >= 3, "not a vacuous filter");
        let one = CanonicalBars::Taken { at: at(), feeds };
        assert!(one.whole_store(tol()).is_none(), "one venue cannot support a claim about every venue");

        let r = one.report(tol());
        assert!(r.with_severity(Severity::Alarm).is_empty(), "no venue-shaped claim from one venue");
        assert!(!r.findings.is_empty(), "the feeds are still reported");
        assert!(!r.all_clear(), "withholding a claim is not a clean bill of health");
    }

    /// A canonical store that holds nothing at all is an alarm, and the one case
    /// where no venue count is needed to defend it: no market closure has ever
    /// emptied a table. Without this the emptiest possible store produced the
    /// emptiest possible report — no findings, nothing to page, nothing to show.
    #[test]
    fn an_empty_canonical_store_alarms_rather_than_reporting_nothing() {
        let empty = CanonicalBars::Taken { at: at(), feeds: Vec::new() };
        let (_, beat) = empty.whole_store(tol()).expect("an empty store is still a claim");
        assert!(beat.is_none(), "there is nothing to derive a clock from");

        let r = empty.report(tol());
        assert_eq!(r.findings.len(), 1, "an empty store is not an empty report");
        assert_eq!(r.findings[0].feed, FeedKey::new(WHOLE_STORE, WHOLE_SERIES, WHOLE_SERIES));
        assert_eq!(r.findings[0].verdict, Verdict::NeverSeen, "never written is not merely stale");
        assert_eq!(r.worst(), Some(Severity::Alarm));
        assert_eq!(r.pages().len(), 1);
    }

    /// A failed census is blind, not healthy, and still names the store.
    #[test]
    fn a_canonical_read_that_failed_is_unobserved_and_never_empty() {
        let r = CanonicalBars::Failed { attempted_at: at(), reason: "connection refused".into() }.report(tol());
        assert_eq!(r.findings.len(), 1);
        assert_eq!(r.findings[0].feed, FeedKey::new(WHOLE_STORE, WHOLE_SERIES, WHOLE_SERIES));
        assert!(matches!(r.findings[0].verdict, Verdict::Unobserved { .. }));
        assert_eq!(r.worst(), Some(Severity::Warn), "our failure to look is not the collector's fault");
        assert!(!r.all_clear());
    }

    /// The whole-store expectation is judged on our clock, never through
    /// `dataplane::retention` — which is why its disk budget is inert and why
    /// there is no `Arrival::Always` anywhere in this module.
    #[test]
    fn the_whole_store_claim_never_consults_retention() {
        let exp = whole_store_expectation(3600, tol());
        assert_eq!(exp.arrival, Arrival::Polled { poll_period_secs: 3600 });
        assert_ne!(exp.arrival, Arrival::Always, "we hold no evidence that every venue is always open");
        assert_eq!(exp.write_deadline_secs(), Some(3 * 3600));

        let src = include_str!("census.rs");
        let production = src.split("#[cfg(test)]").next().unwrap_or(src);
        let code: String =
            production.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
        assert!(!code.contains("Arrival::Always"), "an always-on claim needs a venue guarantee to cite");
    }

    /// A 1-minute feed and an hourly feed on the same instrument are two feeds.
    /// Folding them into one key would let the hourly one's freshness cover the
    /// minute one's death.
    #[test]
    fn two_periods_on_one_instrument_are_two_feeds() {
        assert_ne!(feed_series_id(60), feed_series_id(3600));
        let r = CanonicalBars::Taken { at: at(), feeds: live_feeds() }.report(tol());
        let coinbase_btc: Vec<_> =
            r.findings.iter().filter(|f| f.feed.venue == "coinbase" && f.feed.instrument == "BTC-USD").collect();
        assert_eq!(coinbase_btc.len(), 2, "one minute feed and one hourly feed");
        assert_ne!(coinbase_btc[0].feed.series, coinbase_btc[1].feed.series);
    }

    // ── The registry mapping ─────────────────────────────────────────────────

    /// `dataplane.forward_series` has no arrival column, so there is nothing to
    /// read and the reader says so in code rather than in a comment. When the
    /// column lands this test is the one that fails.
    #[test]
    fn the_registry_declares_no_arrival_pattern_today() {
        assert!(arrival_column().is_none());
        let row = RegistryRow {
            series_id: "kraken_futures.funding".into(),
            store_table: "funding_rates".into(),
            venue: "kraken_futures".into(),
            cadence_secs: 3600,
            alarm_after_cadences: 3,
            disk_budget_bytes: 2_147_483_648,
            retention_policy: "none".into(),
            downsample_after_days: None,
            downsample_to_secs: None,
            enabled: true,
            arrival: arrival_column(),
        };
        let exp = FeedExpectation::from_registry(&row, Roster::WhateverAppears).unwrap();
        assert_eq!(exp.arrival, Arrival::Polled { poll_period_secs: 3600 }, "our cadence, not a venue claim");
    }

    // ── What is still needed ─────────────────────────────────────────────────

    /// The two things that would let a canonical feed be judged individually, held
    /// here so they cannot be quietly forgotten:
    ///
    /// 1. a `dataplane.forward_series` row whose `store_table` is `market_bar`,
    ///    which records the promise;
    /// 2. a call to `storage::clickhouse::forward::beat` from the bar persister on
    ///    every poll, zero rows included, which gives the store a write clock a
    ///    shut market cannot stop.
    ///
    /// Until both exist, per-feed canonical verdicts are `CannotJudge`, and this
    /// test asserts that the code has not started pretending otherwise.
    #[test]
    fn a_canonical_feed_becomes_judgeable_only_with_a_registry_row_and_a_beat() {
        // (1) No expectation in this module names a per-feed canonical series.
        let r = CanonicalBars::Taken { at: at(), feeds: live_feeds() }.report(tol());
        assert!(
            r.findings.iter().filter(|f| f.feed.instrument != WHOLE_SERIES).all(|f| matches!(
                f.verdict,
                Verdict::CannotJudge { .. }
            )),
            "a per-feed canonical deadline was invented somewhere: {:?}",
            r.findings
        );
        // (2) The beat this module is waiting for exists and takes exactly the
        // arguments the bar persister has to hand. Constructed, never awaited:
        // building the future is what proves the call compiles.
        drop(forward::beat("http://127.0.0.1:1/trading", &feed_series_id(60), "coinbase", "BTC-USD", at(), 0));
    }

    /// The worst case: neither store answers. Nothing is mocked — both handles
    /// point at a port nothing listens on, which is the cheapest honest way to
    /// produce a real failure.
    ///
    /// The pass must still produce findings. A silence alarm that returns an empty
    /// report when it cannot reach anything has reproduced, in itself, the exact
    /// failure it was built to catch: a thing that stopped working and looked
    /// identical to a thing with nothing to say.
    #[tokio::test]
    async fn a_pass_that_can_reach_neither_store_still_produces_findings() {
        let pg = sqlx::PgPool::connect_lazy("postgres://nobody:nobody@127.0.0.1:1/none")
            .expect("a lazy pool does not connect");
        let fut = silence_report(&pg, "http://127.0.0.1:1/trading", at(), tol());
        let r = tokio::time::timeout(std::time::Duration::from_secs(40), fut)
            .await
            .expect("the pass should fail fast, not hang");

        assert_eq!(r.findings.len(), 2, "one per store: {:?}", r.findings);
        assert!(
            r.findings.iter().all(|f| matches!(f.verdict, Verdict::Unobserved { .. })),
            "our own outage is never a collector's fault: {:?}",
            r.findings
        );
        assert_eq!(r.worst(), Some(Severity::Warn));
        assert!(r.pages().is_empty(), "nobody is paged for an accusation we cannot make");
        assert!(!r.all_clear(), "blind is not healthy");

        let named: Vec<&str> = r.findings.iter().map(|f| f.feed.series.as_str()).collect();
        assert!(named.contains(&REGISTRY_TABLE), "the registry is named: {named:?}");
        assert!(named.contains(&WHOLE_STORE), "the canonical store is named: {named:?}");
    }

    // ── Against the live stores ──────────────────────────────────────────────

    /// Gated, because it needs both stores. Run with:
    ///
    /// ```text
    /// STORAGE_CENSUS_CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
    /// STORAGE_CENSUS_PG_URL=postgres://trading:trading@localhost:5432/trading \
    ///   cargo test -p storage -j 2 -- --nocapture census
    /// ```
    #[tokio::test]
    async fn against_the_live_stores_the_pass_is_not_silent() {
        let (Ok(ch), Ok(pg_url)) =
            (std::env::var("STORAGE_CENSUS_CLICKHOUSE_URL"), std::env::var("STORAGE_CENSUS_PG_URL"))
        else {
            eprintln!("STORAGE_CENSUS_CLICKHOUSE_URL / STORAGE_CENSUS_PG_URL unset — skipping");
            return;
        };
        let pg = sqlx::PgPool::connect(&pg_url).await.expect("postgres");
        let now = Utc::now();

        let bars = canonical_bar_census(&ch, now).await;
        let CanonicalBars::Taken { feeds, .. } = &bars else { panic!("the canonical census failed: {bars:?}") };
        eprintln!("canonical feeds: {}", feeds.len());
        for f in feeds {
            eprintln!(
                "  {}/{} @{}s  event {}  write {}",
                f.venue, f.instrument, f.period_secs, f.last_event_time, f.last_write_time
            );
        }

        let report = silence_report(&pg, &ch, now, tol()).await;
        observability::silence::emit(&report);
        for f in &report.findings {
            eprintln!("  [{:?}] {}/{}/{} {:?}", f.severity(), f.feed.series, f.feed.venue, f.feed.instrument, f.verdict);
        }
        eprintln!("  pages: {}", report.pages().len());
        assert!(!report.findings.is_empty(), "a pass that asked nothing learned nothing");
        assert!(!report.all_clear(), "nothing in either store has been written today");
        assert_eq!(report.worst(), Some(Severity::Alarm), "the forward series have never produced a row");
    }
}
