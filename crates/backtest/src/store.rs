//! The single point-in-time reader and writer for market bars (SPEC §1.2–§1.3;
//! INV-01, INV-02, INV-04, INV-05, INV-06).
//!
//! Reads and writes `market_bar` (`clickhouse/07_canonical_market.sql`). No other
//! module may name that table or its predecessors; `ci_invariants` fails the build
//! if one does. Every read is point-in-time: rows with `knowledge_time > as_of` are
//! invisible, and a restated cell resolves to the latest version known by `as_of`.
//! There is no non-PIT method, at any level.
//!
//! Callers keep addressing instruments by symbol. Symbols resolve bitemporally
//! through `instrument_symbol_dim` to surrogate ids; storage keys are never symbols.
//! One venue is read at a time — blending venues would fabricate a price nobody
//! traded (SPEC §1.7). Without an explicit venue the reader takes the symbol's
//! best-quality venue (lowest `quality_tier`, then lowest id): a fixed rule, never a
//! data-dependent choice.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use clickhouse::Row;
use dataplane::bar::{self, CanonicalBar, KnowledgeProvenance, VendorLag, PRICE_SCALE};
use dataplane::identity::{AssetClass, InstrumentKey, VenueKey};
use dataplane::quality::QualityFlags;
use domain::payloads::bar::Timeframe;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::types::TimeframeExt;

/// How long after its close a bar may arrive and still count as observed live.
const LIVE_TOLERANCE_SECS: i64 = 300;

/// A resolved bar.
#[derive(Clone, Debug, Default)]
pub struct LoadedBar {
    /// The instant the bar completed (open + period), Unix ns. The earliest moment a
    /// strategy acting on the whole bar could act.
    pub ts_ns: i64,
    /// The bar OPEN, Unix ns — the stored `event_time`.
    pub open_ns: i64,
    /// When the bar became queryable, Unix ns.
    pub knowledge_ns: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: Decimal,
    pub trade_count: u64,
    pub quality_flags: QualityFlags,
}

/// Coverage for one (symbol, timeframe, venue).
#[derive(Clone, Debug)]
pub struct BarCoverage {
    pub instrument_id: String,
    pub venue: String,
    pub timeframe: String,
    pub bars: u64,
    /// First bar close, Unix ns.
    pub first_ns: i64,
    /// Last bar close, Unix ns.
    pub last_ns: i64,
    /// Bars whose knowledge time is a declared backfill sentinel.
    pub backfilled: u64,
}

/// A bar produced by a collector, ready for insert.
#[derive(Clone, Debug)]
pub struct CollectedBar {
    /// Bar close time.
    pub available_time: DateTime<Utc>,
    /// Monotonic sequence (bar open in epoch units). Kept for collector bookkeeping.
    pub sequence: u64,
    /// Decimal strings — never floats.
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub trade_count: u64,
}

/// Data-quality profile for QC jobs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityProfile {
    pub rows: u64,
    pub distinct_bars: u64,
    pub first_s: i64,
    pub last_s: i64,
    pub flat_bars: u64,
    pub backfilled: u64,
    pub flagged: u64,
}

/// Outcome of a write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    pub inserted: usize,
    /// Identical re-collections: already stored, not written again.
    pub unchanged: usize,
    /// Cells whose values changed: written as a new revision and indexed.
    pub restated: usize,
    /// Bars the ingest validator refused.
    pub rejected: usize,
}

pub struct BarStore {
    client: clickhouse::Client,
    /// Point-in-time horizon for every read. `None` is "as of now": a bar is never
    /// visible before the platform could have known it.
    as_of: Option<DateTime<Utc>>,
    venue: Option<String>,
}

#[derive(Row, Deserialize)]
struct FactRow {
    instrument_id: i64,
    venue_id: i32,
    valid_from_ns: i64,
    valid_to_ns: i64,
}

#[derive(Row, Deserialize)]
struct BarRowOut {
    open_ns: i64,
    knowledge_ns: i64,
    out_period: u32,
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
    trade_count: u64,
    quality_flags: u32,
}

#[derive(Row, Serialize)]
struct MarketBarRow {
    instrument_id: i64,
    venue_id: i32,
    period_secs: u32,
    event_time: i64,
    venue_ts: Option<i64>,
    ingest_time: i64,
    knowledge_time: i64,
    open: i128,
    high: i128,
    low: i128,
    close: i128,
    volume: i128,
    trade_count: Option<u32>,
    vwap: Option<i128>,
    bid_close: Option<i128>,
    ask_close: Option<i128>,
    bipower_var: Option<f64>,
    n_updates: Option<u32>,
    quality_flags: u32,
    revision_seq: u32,
    source_id: i32,
}

#[derive(Row, Serialize)]
struct RestatementRow {
    instrument_id: i64,
    venue_id: i32,
    period_secs: u32,
    event_date: u16,
    n_revisions: u32,
    max_knowledge_time: i64,
}

#[derive(Row, Deserialize)]
struct ExistingCell {
    open_ns: i64,
    values: String,
    max_rev: u32,
}

/// One resolved symbol fact: an instrument on a venue over a validity window.
#[derive(Clone, Debug)]
struct Fact {
    instrument: InstrumentKey,
    venue: VenueKey,
    valid_from_ns: i64,
    valid_to_ns: i64,
}

impl BarStore {
    /// Connect using a full URL that may include credentials and a database path
    /// (`http://user:pass@host:8123/dbname`).
    pub fn connect(url: &str) -> Self {
        Self { client: storage::clickhouse::connect(url), as_of: None, venue: None }
    }

    /// Read the world as it stood at `as_of`.
    #[must_use]
    pub fn as_of(mut self, as_of: DateTime<Utc>) -> Self {
        self.as_of = Some(as_of);
        self
    }

    /// Read one named venue instead of the symbol's best-quality venue.
    #[must_use]
    pub fn venue(mut self, venue: impl Into<String>) -> Self {
        self.venue = Some(venue.into());
        self
    }

    fn horizon_ns(&self) -> i64 {
        nanos(self.as_of.unwrap_or_else(Utc::now))
    }

    /// Resolve `symbol` to the instrument(s) it denoted on the chosen venue, using
    /// only identity facts known by the horizon.
    async fn facts(&self, symbol: &str) -> anyhow::Result<Vec<Fact>> {
        let horizon = self.horizon_ns();
        let venue_filter = match &self.venue {
            Some(_) => "AND s.venue_id = (SELECT venue_id FROM venue_dim FINAL WHERE name = ? LIMIT 1)",
            None => "",
        };
        let sql = format!(
            "SELECT instrument_id, venue_id,
                    toInt64(toUnixTimestamp64Nano(valid_from)) AS valid_from_ns,
                    ifNull(toInt64(toUnixTimestamp64Nano(valid_to)), {max}) AS valid_to_ns
               FROM (
                 SELECT s.instrument_id, s.venue_id, s.valid_from,
                        argMax(s.valid_to, s.knowledge_time) AS valid_to
                   FROM instrument_symbol_dim AS s
                  WHERE s.symbol = ? {venue_filter}
                    AND s.knowledge_time <= fromUnixTimestamp64Nano({horizon})
                  GROUP BY s.instrument_id, s.venue_id, s.symbol, s.valid_from
               ) AS f
              ORDER BY venue_id, valid_from",
            max = i64::MAX,
        );
        let mut q = self.client.query(&sql).bind(symbol);
        if let Some(v) = &self.venue {
            q = q.bind(v.as_str());
        }
        let rows: Vec<FactRow> = q.fetch_all().await?;
        let mut facts: Vec<Fact> = rows
            .into_iter()
            .map(|r| Fact {
                instrument: InstrumentKey(r.instrument_id),
                venue: VenueKey(r.venue_id),
                valid_from_ns: r.valid_from_ns,
                valid_to_ns: r.valid_to_ns,
            })
            .collect();
        if self.venue.is_none() {
            if let Some(best) = self.best_venue(&facts).await? {
                facts.retain(|f| f.venue == best);
            }
        }
        Ok(facts)
    }

    async fn best_venue(&self, facts: &[Fact]) -> anyhow::Result<Option<VenueKey>> {
        let ids: BTreeSet<i32> = facts.iter().map(|f| f.venue.0).collect();
        if ids.len() <= 1 {
            return Ok(ids.into_iter().next().map(VenueKey));
        }
        #[derive(Row, Deserialize)]
        struct Tier {
            venue_id: i32,
        }
        let list = ids.iter().map(ToString::to_string).collect::<Vec<_>>().join(",");
        let best: Option<Tier> = self
            .client
            .query(&format!("SELECT venue_id FROM venue_dim FINAL WHERE venue_id IN ({list}) ORDER BY quality_tier, venue_id LIMIT 1"))
            .fetch_optional()
            .await?;
        Ok(best.map(|b| VenueKey(b.venue_id)))
    }

    /// The surrogate `(instrument_id, venue_id)` a symbol currently reads from, under
    /// the same venue rule as every bar read.
    ///
    /// # Errors
    /// Backend failures.
    pub async fn resolve_instrument(&self, symbol: &str) -> anyhow::Result<Option<(i64, i32)>> {
        let facts = self.facts(symbol).await?;
        Ok(facts.iter().max_by_key(|f| f.valid_from_ns).map(|f| (f.instrument.0, f.venue.0)))
    }

    /// The core PIT read. `from`/`to` bound bar CLOSE times, `[from, to)`.
    async fn read(&self, symbol: &str, period_secs: u32, from_close_ns: i64, to_close_ns: i64) -> anyhow::Result<Vec<LoadedBar>> {
        let facts = self.facts(symbol).await?;
        self.read_facts(&facts, period_secs, from_close_ns, to_close_ns).await
    }

    /// Bars of one surrogate `(instrument, venue)` whose close falls in
    /// `[from_close_ns, to_close_ns)`, through the same point-in-time read as every
    /// symbol read — for callers that already hold the surrogate key (a logged
    /// feature serve being recomputed, say).
    ///
    /// # Errors
    /// Backend failures.
    pub async fn load_bars_by_key(&self, instrument_id: i64, venue_id: i32, period_secs: u32, from_close_ns: i64, to_close_ns: i64) -> anyhow::Result<Vec<LoadedBar>> {
        let fact = Fact { instrument: InstrumentKey(instrument_id), venue: VenueKey(venue_id), valid_from_ns: i64::MIN, valid_to_ns: i64::MAX };
        self.read_facts(std::slice::from_ref(&fact), period_secs, from_close_ns, to_close_ns).await
    }

    async fn read_facts(&self, facts: &[Fact], period_secs: u32, from_close_ns: i64, to_close_ns: i64) -> anyhow::Result<Vec<LoadedBar>> {
        if facts.is_empty() {
            return Ok(Vec::new());
        }
        let horizon = self.horizon_ns();
        let period_ns = i64::from(period_secs) * 1_000_000_000;
        let from_open = from_close_ns.saturating_sub(period_ns);
        let to_open = to_close_ns.saturating_sub(period_ns);
        let mut out = Vec::new();
        for f in facts {
            let lo = from_open.max(f.valid_from_ns);
            let hi = to_open.min(f.valid_to_ns);
            if lo >= hi {
                continue;
            }
            let cell = format!(
                "instrument_id = {i} AND venue_id = {v} AND period_secs = {period_secs}
                 AND event_time >= fromUnixTimestamp64Nano({lo}) AND event_time < fromUnixTimestamp64Nano({hi})
                 AND knowledge_time <= fromUnixTimestamp64Nano({horizon})",
                i = f.instrument.0,
                v = f.venue.0,
            );
            // §1.3: consult the (small) restatement index first. Partitions absent from
            // it have one version per cell and skip resolution entirely.
            #[derive(Row, Deserialize)]
            struct RestatedDate {
                d: String,
            }
            let restated_dates: Vec<RestatedDate> = self
                .client
                .query(&format!(
                    "SELECT DISTINCT toString(event_date) AS d FROM restatement_index
                      WHERE instrument_id = {i} AND venue_id = {v} AND period_secs = {period_secs}
                        AND event_date >= toDate(fromUnixTimestamp64Nano({lo})) AND event_date <= toDate(fromUnixTimestamp64Nano({hi}))",
                    i = f.instrument.0,
                    v = f.venue.0,
                ))
                .fetch_all()
                .await?;
            let fast = format!(
                "SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS open_ns,
                        toInt64(toUnixTimestamp64Nano(knowledge_time)) AS knowledge_ns,
                        toUInt32({period_secs}) AS out_period,
                        toString(open) AS open, toString(high) AS high, toString(low) AS low,
                        toString(close) AS close, toString(volume) AS volume,
                        toUInt64(ifNull(trade_count, 0)) AS trade_count, quality_flags
                   FROM market_bar
                  WHERE {cell} {{exclude}}
                  ORDER BY event_time, knowledge_time, revision_seq"
            );
            let sql = if restated_dates.is_empty() {
                fast.replace("{exclude}", "")
            } else {
                let list = restated_dates.iter().map(|r| format!("'{}'", r.d)).collect::<Vec<_>>().join(",");
                format!(
                    "SELECT * FROM (
                       {fast_part}
                       UNION ALL
                       SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS open_ns,
                              toInt64(toUnixTimestamp64Nano(argMax(knowledge_time, (knowledge_time, revision_seq)))) AS knowledge_ns,
                              toUInt32({period_secs}) AS out_period,
                              argMax(toString(open), (knowledge_time, revision_seq)) AS open,
                              argMax(toString(high), (knowledge_time, revision_seq)) AS high,
                              argMax(toString(low), (knowledge_time, revision_seq)) AS low,
                              argMax(toString(close), (knowledge_time, revision_seq)) AS close,
                              argMax(toString(volume), (knowledge_time, revision_seq)) AS volume,
                              toUInt64(ifNull(argMax(trade_count, (knowledge_time, revision_seq)), 0)) AS trade_count,
                              argMax(quality_flags, (knowledge_time, revision_seq)) AS quality_flags
                         FROM market_bar
                        WHERE {cell} AND toDate(event_time) IN ({list})
                        GROUP BY event_time
                     ) ORDER BY open_ns, knowledge_ns",
                    fast_part = fast.replace("{exclude}", &format!("AND toDate(event_time) NOT IN ({list})")),
                )
            };
            let rows: Vec<BarRowOut> = self.client.query(&sql).fetch_all().await?;
            // Rows arrive in primary-key order (read-in-order, no sort). Unmerged exact
            // re-collections share an event_time; the last row per cell is its latest
            // version, so each cell keeps only that one.
            let mut rows = rows;
            rows.dedup_by(|later, earlier| {
                if later.open_ns == earlier.open_ns {
                    std::mem::swap(later, earlier);
                    true
                } else {
                    false
                }
            });
            for r in rows {
                out.push(LoadedBar {
                    ts_ns: r.open_ns + i64::from(r.out_period) * 1_000_000_000,
                    open_ns: r.open_ns,
                    knowledge_ns: r.knowledge_ns,
                    open: r.open.parse()?,
                    high: r.high.parse()?,
                    low: r.low.parse()?,
                    close: r.close.parse()?,
                    volume: r.volume.parse()?,
                    trade_count: r.trade_count,
                    quality_flags: QualityFlags(r.quality_flags),
                });
            }
        }
        out.sort_by_key(|b| b.open_ns);
        Ok(out)
    }

    /// Resolved bars whose close falls in `[from, to)`, ordered by time.
    pub async fn load_bars(&self, instrument_id: &str, timeframe: Timeframe, from: DateTime<Utc>, to: DateTime<Utc>) -> anyhow::Result<Vec<LoadedBar>> {
        self.read(instrument_id, period_of(timeframe)?, nanos(from), nanos(to)).await
    }

    /// Bars rolled up from a finer stored timeframe into `bucket_seconds` candles.
    /// Built on the same PIT read, so a rollup can never see what a plain read cannot.
    pub async fn load_bars_bucketed(
        &self,
        instrument_id: &str,
        base_timeframe: Timeframe,
        bucket_seconds: u32,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> anyhow::Result<Vec<LoadedBar>> {
        let bars = self.load_bars(instrument_id, base_timeframe, from, to).await?;
        let bucket_ns = i64::from(bucket_seconds.max(1)) * 1_000_000_000;
        let mut out: Vec<LoadedBar> = Vec::new();
        for b in bars {
            let start = b.open_ns.div_euclid(bucket_ns) * bucket_ns;
            match out.last_mut() {
                Some(cur) if cur.open_ns == start => {
                    cur.high = cur.high.max(b.high);
                    cur.low = cur.low.min(b.low);
                    cur.close = b.close;
                    cur.volume += b.volume;
                    cur.trade_count += b.trade_count;
                    cur.knowledge_ns = cur.knowledge_ns.max(b.knowledge_ns);
                    cur.quality_flags |= b.quality_flags;
                }
                _ => out.push(LoadedBar { open_ns: start, ts_ns: start, ..b }),
            }
        }
        Ok(out)
    }

    /// Per-day distinct bar counts (by bar close date).
    pub async fn daily_counts(&self, instrument_id: &str, timeframe: Timeframe, from: DateTime<Utc>, to: DateTime<Utc>) -> anyhow::Result<HashMap<NaiveDate, u64>> {
        let bars = self.load_bars(instrument_id, timeframe, from, to).await?;
        let mut out: HashMap<NaiveDate, u64> = HashMap::new();
        for b in bars {
            *out.entry(DateTime::from_timestamp_nanos(b.ts_ns).date_naive()).or_default() += 1;
        }
        Ok(out)
    }

    /// Every (symbol, venue, timeframe) with bars visible at the horizon.
    pub async fn list_coverage(&self) -> anyhow::Result<Vec<BarCoverage>> {
        #[derive(Row, Deserialize)]
        struct CoverageRow {
            symbol: String,
            venue: String,
            period_secs: u32,
            bars: u64,
            first_ns: i64,
            last_ns: i64,
            backfilled: u64,
        }
        let horizon = self.horizon_ns();
        let rows: Vec<CoverageRow> = self
            .client
            .query(&format!(
                "SELECT s.symbol AS symbol, v.name AS venue, m.period_secs AS period_secs,
                        toUInt64(uniqExact(m.event_time)) AS bars,
                        toInt64(toUnixTimestamp64Nano(min(m.event_time))) + toInt64(m.period_secs) * 1000000000 AS first_ns,
                        toInt64(toUnixTimestamp64Nano(max(m.event_time))) + toInt64(m.period_secs) * 1000000000 AS last_ns,
                        toUInt64(countIf(bitAnd(m.quality_flags, {bf}) != 0)) AS backfilled
                   FROM market_bar AS m
                   INNER JOIN (SELECT DISTINCT instrument_id, venue_id, symbol FROM instrument_symbol_dim FINAL
                                WHERE knowledge_time <= fromUnixTimestamp64Nano({horizon})) AS s
                     ON m.instrument_id = s.instrument_id AND m.venue_id = s.venue_id
                   INNER JOIN (SELECT venue_id, name FROM venue_dim FINAL) AS v ON m.venue_id = v.venue_id
                  WHERE m.knowledge_time <= fromUnixTimestamp64Nano({horizon})
                  GROUP BY s.symbol, v.name, m.period_secs
                  ORDER BY s.symbol, v.name, m.period_secs",
                bf = QualityFlags::BACKFILLED_KNOWLEDGE_TIME.0,
            ))
            .fetch_all()
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| BarCoverage {
                instrument_id: r.symbol,
                venue: r.venue,
                timeframe: timeframe_key(r.period_secs),
                bars: r.bars,
                first_ns: r.first_ns,
                last_ns: r.last_ns,
                backfilled: r.backfilled,
            })
            .collect())
    }

    /// Close time of the latest visible bar.
    pub async fn last_bar_time(&self, instrument_id: &str, timeframe: Timeframe) -> anyhow::Result<Option<DateTime<Utc>>> {
        let facts = self.facts(instrument_id).await?;
        if facts.is_empty() {
            return Ok(None);
        }
        #[derive(Row, Deserialize)]
        struct MaxTs {
            open_ns: i64,
            n: u64,
        }
        let period = period_of(timeframe)?;
        let horizon = self.horizon_ns();
        let mut best: Option<i64> = None;
        for f in &facts {
            let row: MaxTs = self
                .client
                .query(&format!(
                    "SELECT toInt64(toUnixTimestamp64Nano(max(event_time))) AS open_ns, count() AS n
                       FROM market_bar
                      WHERE instrument_id = {i} AND venue_id = {v} AND period_secs = {period}
                        AND knowledge_time <= fromUnixTimestamp64Nano({horizon})",
                    i = f.instrument.0,
                    v = f.venue.0,
                ))
                .fetch_one()
                .await?;
            if row.n > 0 {
                best = Some(best.map_or(row.open_ns, |b| b.max(row.open_ns)));
            }
        }
        Ok(best.map(|open| DateTime::from_timestamp_nanos(open + i64::from(period) * 1_000_000_000)))
    }

    /// Data-quality profile for one symbol and timeframe at the horizon.
    pub async fn quality_profile(&self, instrument_id: &str, timeframe: Timeframe) -> anyhow::Result<QualityProfile> {
        let facts = self.facts(instrument_id).await?;
        let period = period_of(timeframe)?;
        let horizon = self.horizon_ns();
        let mut p = QualityProfile::default();
        for f in &facts {
            #[derive(Row, Deserialize)]
            struct Q {
                rows: u64,
                distinct_bars: u64,
                first_s: i64,
                last_s: i64,
                flat_bars: u64,
                backfilled: u64,
                flagged: u64,
            }
            let q: Q = self
                .client
                .query(&format!(
                    "SELECT toUInt64(count()) AS rows, toUInt64(uniqExact(event_time)) AS distinct_bars,
                            toInt64(toUnixTimestamp(min(event_time))) AS first_s,
                            toInt64(toUnixTimestamp(max(event_time))) AS last_s,
                            toUInt64(countIf(high = low)) AS flat_bars,
                            toUInt64(countIf(bitAnd(quality_flags, {bf}) != 0)) AS backfilled,
                            toUInt64(countIf(bitAnd(quality_flags, bitNot(toUInt32({bf}))) != 0)) AS flagged
                       FROM market_bar
                      WHERE instrument_id = {i} AND venue_id = {v} AND period_secs = {period}
                        AND knowledge_time <= fromUnixTimestamp64Nano({horizon})",
                    bf = QualityFlags::BACKFILLED_KNOWLEDGE_TIME.0,
                    i = f.instrument.0,
                    v = f.venue.0,
                ))
                .fetch_one()
                .await?;
            if q.rows == 0 {
                continue;
            }
            p.first_s = if p.rows == 0 { q.first_s } else { p.first_s.min(q.first_s) };
            p.last_s = p.last_s.max(q.last_s);
            p.rows += q.rows;
            p.distinct_bars += q.distinct_bars;
            p.flat_bars += q.flat_bars;
            p.backfilled += q.backfilled;
            p.flagged += q.flagged;
        }
        Ok(p)
    }

    /// Insert collected bars with honest provenance.
    ///
    /// * identity: `(venue, symbol)` resolves to a surrogate id (allocated if new);
    /// * convention: stored `event_time` is the bar OPEN, validated at ingest;
    /// * knowledge: a bar from a live source arriving within the tolerance is
    ///   observed (`knowledge_time` = receipt); anything else is a backfill and gets
    ///   the source's declared vendor lag plus `BACKFILLED_KNOWLEDGE_TIME`;
    /// * restatement: an identical re-collection writes nothing; a changed cell
    ///   becomes a new `revision_seq`, flagged `VENDOR_REVISED`, and indexed.
    ///
    /// # Errors
    /// No identity resolver installed, unknown venue or source, or a backend failure.
    pub async fn insert_collected(
        &self,
        instrument_id: &str,
        venue_id: &str,
        source: &str,
        _trust_tier: &str,
        timeframe: Timeframe,
        bars: &[CollectedBar],
    ) -> anyhow::Result<()> {
        self.write_collected(instrument_id, venue_id, source, timeframe, bars).await.map(|_| ())
    }

    /// As [`Self::insert_collected`], returning what happened.
    ///
    /// # Errors
    /// See [`Self::insert_collected`].
    pub async fn write_collected(&self, symbol: &str, venue: &str, source: &str, timeframe: Timeframe, bars: &[CollectedBar]) -> anyhow::Result<WriteReport> {
        let mut report = WriteReport::default();
        if bars.is_empty() {
            return Ok(report);
        }
        let identity = storage::identity::installed()
            .ok_or_else(|| anyhow::anyhow!("bar writes require an installed identity resolver (storage::identity::install)"))?;
        let period = period_of(timeframe)?;
        let period_ns = i64::from(period) * 1_000_000_000;
        let first_open = bars.iter().map(|b| nanos(b.available_time) - period_ns).min().unwrap_or(0);
        let last_open = bars.iter().map(|b| nanos(b.available_time) - period_ns).max().unwrap_or(0);
        let (instrument, venue_key) = identity
            .ensure_instrument(venue, symbol, asset_class_guess(symbol), DateTime::from_timestamp_nanos(first_open))
            .await?;
        let src = identity.source(source).await?;
        let lag = VendorLag { source_id: src.source_id, lag: src.declared_vendor_lag };
        let ingest = Utc::now();

        // Existing cells in range, for idempotence and restatement detection.
        let existing: Vec<ExistingCell> = self
            .client
            .query(&format!(
                "SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS open_ns,
                        argMax(concat(toString(open),'|',toString(high),'|',toString(low),'|',toString(close),'|',toString(volume)),
                               (knowledge_time, revision_seq)) AS values,
                        max(revision_seq) AS max_rev
                   FROM market_bar
                  WHERE instrument_id = {i} AND venue_id = {v} AND period_secs = {period}
                    AND event_time >= fromUnixTimestamp64Nano({first_open}) AND event_time <= fromUnixTimestamp64Nano({last_open})
                  GROUP BY event_time",
                i = instrument.0,
                v = venue_key.0,
            ))
            .fetch_all()
            .await?;
        let existing: HashMap<i64, (String, u32)> = existing.into_iter().map(|c| (c.open_ns, (c.values, c.max_rev))).collect();

        let mut rows = Vec::with_capacity(bars.len());
        let mut restated_dates: HashMap<NaiveDate, (u32, i64)> = HashMap::new();
        for b in bars {
            let open_time = b.available_time - Duration::nanoseconds(period_ns);
            let parse = |s: &str| -> anyhow::Result<Decimal> { Ok(numeric(s)?.parse::<Decimal>()?) };
            let mut cb = CanonicalBar {
                instrument_id: instrument,
                venue_id: venue_key,
                period_secs: period,
                event_time: open_time,
                venue_ts: None,
                ingest_time: ingest,
                knowledge_time: ingest,
                open: parse(&b.open)?.round_dp(PRICE_SCALE),
                high: parse(&b.high)?.round_dp(PRICE_SCALE),
                low: parse(&b.low)?.round_dp(PRICE_SCALE),
                close: parse(&b.close)?.round_dp(PRICE_SCALE),
                volume: parse(&b.volume)?.round_dp(PRICE_SCALE),
                trade_count: u32::try_from(b.trade_count).ok(),
                vwap: None,
                bid_close: None,
                ask_close: None,
                quality_flags: QualityFlags::NONE,
                revision_seq: 0,
                source_id: src.source_id,
            };
            let provenance = if src.live { bar::provenance_for(&cb, Duration::seconds(LIVE_TOLERANCE_SECS)) } else { KnowledgeProvenance::Backfilled };
            bar::stamp_knowledge_time(&mut cb, provenance, &lag);
            if bar::validate(&cb).is_err() {
                report.rejected += 1;
                continue;
            }
            let open_ns = nanos(cb.event_time);
            let values = format!("{}|{}|{}|{}|{}", fmt18(cb.open), fmt18(cb.high), fmt18(cb.low), fmt18(cb.close), fmt18(cb.volume));
            if let Some((prev, max_rev)) = existing.get(&open_ns) {
                if same_values(prev, &values) {
                    report.unchanged += 1;
                    continue;
                }
                cb.revision_seq = max_rev + 1;
                cb.quality_flags |= QualityFlags::VENDOR_REVISED;
                let e = restated_dates.entry(cb.event_time.date_naive()).or_insert((0, 0));
                e.0 += 1;
                e.1 = e.1.max(nanos(cb.knowledge_time));
                report.restated += 1;
            } else {
                report.inserted += 1;
            }
            rows.push(MarketBarRow {
                instrument_id: instrument.0,
                venue_id: venue_key.0,
                period_secs: period,
                event_time: open_ns,
                venue_ts: None,
                ingest_time: nanos(cb.ingest_time),
                knowledge_time: nanos(cb.knowledge_time),
                open: mantissa(cb.open)?,
                high: mantissa(cb.high)?,
                low: mantissa(cb.low)?,
                close: mantissa(cb.close)?,
                volume: mantissa(cb.volume)?,
                trade_count: cb.trade_count,
                vwap: None,
                bid_close: None,
                ask_close: None,
                bipower_var: None,
                n_updates: None,
                quality_flags: cb.quality_flags.0,
                revision_seq: cb.revision_seq,
                source_id: src.source_id,
            });
        }

        for chunk in rows.chunks(2_000) {
            let mut insert = self.client.insert("market_bar")?;
            for row in chunk {
                insert.write(row).await?;
            }
            insert.end().await?;
        }
        if !restated_dates.is_empty() {
            let mut insert = self.client.insert("restatement_index")?;
            for (date, (n, max_k)) in restated_dates {
                let days = (date - NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch")).num_days();
                insert
                    .write(&RestatementRow {
                        instrument_id: instrument.0,
                        venue_id: venue_key.0,
                        period_secs: period,
                        event_date: u16::try_from(days).unwrap_or(u16::MAX),
                        n_revisions: n,
                        max_knowledge_time: max_k,
                    })
                    .await?;
            }
            insert.end().await?;
        }
        Ok(report)
    }
}

fn nanos(t: DateTime<Utc>) -> i64 {
    t.timestamp_nanos_opt().unwrap_or(0)
}

fn period_of(tf: Timeframe) -> anyhow::Result<u32> {
    u32::try_from(tf.seconds()).map_err(|_| anyhow::anyhow!("timeframe {} does not fit in u32 seconds", tf.key()))
}

fn timeframe_key(period_secs: u32) -> String {
    match period_secs {
        1 => "1s".into(),
        60 => "1m".into(),
        300 => "5m".into(),
        900 => "15m".into(),
        3600 => "1h".into(),
        14_400 => "4h".into(),
        86_400 => "1d".into(),
        other => format!("{other}s"),
    }
}

fn asset_class_guess(symbol: &str) -> AssetClass {
    if symbol.contains('-') || symbol.contains('/') {
        AssetClass::Crypto
    } else {
        AssetClass::Equity
    }
}

/// Unscaled i128 mantissa for a `Decimal(38,18)` column (value × 10^18).
fn mantissa(d: Decimal) -> anyhow::Result<i128> {
    let scaled = d.round_dp(PRICE_SCALE);
    scaled
        .mantissa()
        .checked_mul(10_i128.pow(PRICE_SCALE - scaled.scale()))
        .ok_or_else(|| anyhow::anyhow!("decimal {d} out of Decimal(38,18) range"))
}

fn fmt18(d: Decimal) -> String {
    d.normalize().to_string()
}

/// ClickHouse renders `Decimal(38,18)` with trailing zeros trimmed; compare numerically.
fn same_values(stored: &str, incoming: &str) -> bool {
    let a: Vec<Option<Decimal>> = stored.split('|').map(|s| s.parse::<Decimal>().ok()).collect();
    let b: Vec<Option<Decimal>> = incoming.split('|').map(|s| s.parse::<Decimal>().ok()).collect();
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| matches!((x, y), (Some(x), Some(y)) if x.normalize() == y.normalize()))
}

/// A plain decimal literal from the collector boundary — never an expression.
fn numeric(s: &str) -> anyhow::Result<&str> {
    let ok = !s.is_empty()
        && s.chars().enumerate().all(|(i, c)| c.is_ascii_digit() || c == '.' || (i == 0 && c == '-'))
        && s.chars().filter(|&c| c == '.').count() <= 1;
    anyhow::ensure!(ok, "invalid numeric literal from collector: {s:?}");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn mantissa_is_value_times_1e18() {
        assert_eq!(mantissa(dec!(1)).unwrap(), 1_000_000_000_000_000_000);
        assert_eq!(mantissa(dec!(64449.5)).unwrap(), 64_449_500_000_000_000_000_000);
        assert_eq!(mantissa(dec!(0.00000001)).unwrap(), 10_000_000_000);
        assert_eq!(mantissa(dec!(-2.5)).unwrap(), -2_500_000_000_000_000_000);
    }

    #[test]
    fn values_compare_numerically() {
        assert!(same_values("100|101.5|99|100.25|3", "100.000|101.50|99.0|100.250|3.0"));
        assert!(!same_values("100|101.5|99|100.25|3", "100|101.5|99|100.26|3"));
    }

    #[test]
    fn numeric_rejects_injection() {
        assert!(numeric("123.45").is_ok());
        assert!(numeric("-0.5").is_ok());
        assert!(numeric("1e5").is_err());
        assert!(numeric("1); DROP TABLE market_bar;--").is_err());
        assert!(numeric("").is_err());
    }

    #[test]
    fn timeframe_keys_round_trip() {
        assert_eq!(timeframe_key(60), "1m");
        assert_eq!(timeframe_key(3600), "1h");
        assert_eq!(timeframe_key(42), "42s");
    }
}
