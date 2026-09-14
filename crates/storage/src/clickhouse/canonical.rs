//! One-time migration of `market_bars_v2` into the canonical `market_bar`
//! (clickhouse/07_canonical_market.sql).
//!
//! What changes on the way across, per row:
//! * identity: `(venue name, symbol)` → surrogate `(instrument_id, venue_id)`;
//! * convention: `event_time` becomes the bar OPEN (v2's `bar_open_time`);
//! * precision: Decimal(38,10) → Decimal(38,18);
//! * knowledge: a row from a live source that arrived within five minutes of its
//!   close keeps its observed knowledge time; every other row — REST backfills, which
//!   v2 stamped as knowable at the close — gets `close + declared vendor lag` and
//!   `BACKFILLED_KNOWLEDGE_TIME` (CLAUDE.md §6). Nothing is fabricated.
//!
//! Chunked by (timeframe, month), recorded in `canonical_migration_chunk`, verified
//! per chunk by distinct-cell counts, and skipped once done.

use std::collections::BTreeSet;

use clickhouse::Row;
use serde::Deserialize;
use sqlx::PgPool;
use tracing::info;

use super::ChError;
use crate::identity::{IdentityResolver, PgIdentityService};
use dataplane::identity::AssetClass;

const BACKFILLED: u32 = dataplane::quality::QualityFlags::BACKFILLED_KNOWLEDGE_TIME.0;
const LIVE_TOLERANCE_SECS: i64 = 300;
/// `unknown_legacy` in dataplane.source.
const UNKNOWN_SOURCE_ID: i32 = 6;
const UNKNOWN_SOURCE_LAG_MS: i64 = 60_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalReport {
    pub identities: usize,
    pub chunks_copied: usize,
    pub chunks_skipped: usize,
    pub rows_copied: u64,
    pub backfilled_rows: u64,
}

fn ch_err(ctx: &str, e: impl std::fmt::Display) -> ChError {
    ChError::Client(format!("{ctx}: {e}"))
}

fn guess_class(symbol: &str, venue: &str) -> AssetClass {
    match venue {
        "alpaca" => AssetClass::Equity,
        "oanda" => AssetClass::Fx,
        "cme" => AssetClass::Future,
        "opra" => AssetClass::Option,
        "kalshi" => AssetClass::PredictionMarket,
        _ if symbol.contains('-') || symbol.contains('/') => AssetClass::Crypto,
        _ => AssetClass::Crypto,
    }
}

/// Copy v2 into the canonical table. Idempotent across boots.
///
/// # Errors
/// Backend failures, or a chunk whose verification fails.
pub async fn migrate_to_canonical(ch_url: &str, pg: &PgPool) -> Result<CanonicalReport, ChError> {
    let client = super::connect(ch_url);
    let mut report = CanonicalReport::default();

    #[derive(Row, Deserialize)]
    struct Exists {
        n: u64,
    }
    let has_v2: Exists = client
        .query("SELECT count() AS n FROM system.tables WHERE database = currentDatabase() AND name = 'market_bars_v2'")
        .fetch_one()
        .await
        .map_err(|e| ch_err("probe v2", e))?;
    if has_v2.n == 0 {
        return Ok(report);
    }

    // 1. Identity for every (venue, symbol) v2 has ever seen.
    #[derive(Row, Deserialize)]
    struct Pair {
        venue: String,
        symbol: String,
    }
    let pairs: Vec<Pair> = client
        .query("SELECT DISTINCT venue_id AS venue, instrument_id AS symbol FROM market_bars_v2")
        .fetch_all()
        .await
        .map_err(|e| ch_err("distinct pairs", e))?;
    let identity = PgIdentityService::new(pg.clone(), ch_url);
    for p in &pairs {
        let venue = sqlx::query_scalar::<_, i32>("SELECT venue_id FROM dataplane.venue WHERE name = $1")
            .bind(&p.venue)
            .fetch_optional(pg)
            .await
            .map_err(|e| ch_err("venue lookup", e))?;
        let venue_name = if venue.is_some() { p.venue.as_str() } else { "unknown" };
        identity
            .ensure_instrument(venue_name, &p.symbol, guess_class(&p.symbol, &p.venue), chrono::Utc::now())
            .await
            .map_err(|e| ch_err("ensure identity", e))?;
    }
    report.identities = identity.sync_dims().await.map_err(|e| ch_err("sync dims", e))?;

    // 2. Source map, inlined as a literal table (Postgres is the system of record).
    let sources: Vec<(String, i32, i64, String)> =
        sqlx::query_as("SELECT name, source_id, declared_vendor_lag_ms, kind FROM dataplane.source")
            .fetch_all(pg)
            .await
            .map_err(|e| ch_err("sources", e))?;
    let source_values = sources
        .iter()
        .map(|(name, id, lag, kind)| {
            format!("('{}', {id}, {lag}, {})", name.replace('\'', "''"), u8::from(kind == "live_stream"))
        })
        .collect::<Vec<_>>()
        .join(", ");

    // 3. Chunks.
    #[derive(Row, Deserialize)]
    struct Chunk {
        timeframe: String,
        month: u32,
    }
    let chunks: Vec<Chunk> = client
        .query("SELECT DISTINCT timeframe, toYYYYMM(event_time) AS month FROM market_bars_v2 ORDER BY timeframe, month")
        .fetch_all()
        .await
        .map_err(|e| ch_err("chunks", e))?;
    let done: BTreeSet<(String, u32)> = {
        #[derive(Row, Deserialize)]
        struct Done {
            timeframe: String,
            month: u32,
        }
        client
            .query("SELECT timeframe, month FROM canonical_migration_chunk FINAL WHERE source_table = 'market_bars_v2'")
            .fetch_all::<Done>()
            .await
            .map_err(|e| ch_err("done chunks", e))?
            .into_iter()
            .map(|d| (d.timeframe, d.month))
            .collect()
    };

    for c in chunks {
        if done.contains(&(c.timeframe.clone(), c.month)) {
            report.chunks_skipped += 1;
            continue;
        }
        let tf = c.timeframe.replace('\'', "''");
        let month = c.month;
        let live_cond = format!("src.live = 1 AND v.ingested_time <= v.event_time + INTERVAL {LIVE_TOLERANCE_SECS} SECOND");
        let insert = format!(
            "INSERT INTO market_bar
             SELECT s.instrument_id, vd.venue_id,
                    toUInt32(dateDiff('second', v.bar_open_time, v.event_time)) AS period_secs,
                    v.bar_open_time AS event_time,
                    CAST(NULL AS Nullable(DateTime64(9, 'UTC'))) AS venue_ts,
                    v.ingested_time AS ingest_time,
                    if({live_cond}, greatest(v.ingested_time, v.event_time),
                       v.event_time + toIntervalMillisecond(if(src.source_id = 0, {UNKNOWN_SOURCE_LAG_MS}, src.lag_ms))) AS knowledge_time,
                    CAST(v.open AS Decimal(38, 18)), CAST(v.high AS Decimal(38, 18)), CAST(v.low AS Decimal(38, 18)),
                    CAST(v.close AS Decimal(38, 18)), CAST(v.volume AS Decimal(38, 18)),
                    CAST(toUInt32(least(v.trade_count, 4294967295)) AS Nullable(UInt32)) AS trade_count,
                    CAST(NULL AS Nullable(Decimal(38, 18))), CAST(NULL AS Nullable(Decimal(38, 18))), CAST(NULL AS Nullable(Decimal(38, 18))),
                    CAST(NULL AS Nullable(Float64)), CAST(NULL AS Nullable(UInt32)),
                    if({live_cond}, toUInt32(0), toUInt32({BACKFILLED})) AS quality_flags,
                    v.revision AS revision_seq,
                    if(src.source_id = 0, {UNKNOWN_SOURCE_ID}, src.source_id) AS source_id
               FROM (SELECT * FROM market_bars_v2 FINAL WHERE timeframe = '{tf}' AND toYYYYMM(event_time) = {month}) AS v
               INNER JOIN (SELECT venue_id, name FROM venue_dim FINAL) AS vd
                       ON vd.name = if(v.venue_id IN (SELECT name FROM venue_dim FINAL), v.venue_id, 'unknown')
               INNER JOIN (SELECT DISTINCT instrument_id, venue_id, symbol FROM instrument_symbol_dim FINAL) AS s
                       ON s.symbol = v.instrument_id AND s.venue_id = vd.venue_id
               LEFT JOIN (SELECT * FROM values('name String, source_id Int32, lag_ms Int64, live UInt8', {source_values})) AS src
                       ON src.name = v.source"
        );
        client.query(&insert).execute().await.map_err(|e| ch_err("copy chunk", e))?;

        // Verify: every distinct v2 cell has a canonical cell.
        #[derive(Row, Deserialize)]
        struct Cells {
            n: u64,
        }
        let src_cells: Cells = client
            .query(&format!(
                "SELECT uniqExact(instrument_id, venue_id, event_time, revision) AS n FROM market_bars_v2
                  WHERE timeframe = '{tf}' AND toYYYYMM(event_time) = {month}"
            ))
            .fetch_one()
            .await
            .map_err(|e| ch_err("verify source", e))?;
        let missing: Cells = client
            .query(&format!(
                "SELECT count() AS n FROM (
                   SELECT DISTINCT s.instrument_id AS i, vd.venue_id AS vid, v.bar_open_time AS t, v.revision AS r
                     FROM (SELECT instrument_id, venue_id, bar_open_time, revision FROM market_bars_v2
                            WHERE timeframe = '{tf}' AND toYYYYMM(event_time) = {month}) AS v
                     INNER JOIN (SELECT venue_id, name FROM venue_dim FINAL) AS vd
                             ON vd.name = if(v.venue_id IN (SELECT name FROM venue_dim FINAL), v.venue_id, 'unknown')
                     INNER JOIN (SELECT DISTINCT instrument_id, venue_id, symbol FROM instrument_symbol_dim FINAL) AS s
                             ON s.symbol = v.instrument_id AND s.venue_id = vd.venue_id
                 ) AS want
                 LEFT ANTI JOIN (SELECT DISTINCT instrument_id AS i, venue_id AS vid, event_time AS t, revision_seq AS r FROM market_bar) AS have
                 USING (i, vid, t, r)"
            ))
            .fetch_one()
            .await
            .map_err(|e| ch_err("verify dest", e))?;
        if missing.n > 0 {
            return Err(ChError::Client(format!(
                "canonical migration chunk {tf}/{month}: {} of {} source cells missing after copy",
                missing.n, src_cells.n
            )));
        }

        // Index restated partitions: cells with more than one distinct version.
        client
            .query(&format!(
                "INSERT INTO restatement_index
                 SELECT instrument_id, venue_id, period_secs, toDate(event_time) AS event_date,
                        toUInt32(sum(versions - 1)) AS n_revisions, max(mk) AS max_knowledge_time
                   FROM (SELECT instrument_id, venue_id, period_secs, event_time,
                                uniqExact(knowledge_time, revision_seq) AS versions, max(knowledge_time) AS mk
                           FROM market_bar
                          WHERE toYYYYMM(event_time + toIntervalSecond(period_secs)) = {month}
                          GROUP BY instrument_id, venue_id, period_secs, event_time
                         HAVING versions > 1)
                  GROUP BY instrument_id, venue_id, period_secs, event_date"
            ))
            .execute()
            .await
            .map_err(|e| ch_err("restatement index", e))?;

        #[derive(Row, Deserialize)]
        struct Backfilled {
            n: u64,
        }
        let bf: Backfilled = client
            .query(&format!(
                "SELECT countIf(bitAnd(quality_flags, {BACKFILLED}) != 0) AS n FROM market_bar
                  WHERE toYYYYMM(event_time + toIntervalSecond(period_secs)) = {month}"
            ))
            .fetch_one()
            .await
            .map_err(|e| ch_err("count backfilled", e))?;
        report.backfilled_rows = bf.n.max(report.backfilled_rows);

        client
            .query(&format!(
                "INSERT INTO canonical_migration_chunk (source_table, timeframe, month, rows_copied, copied_at)
                 VALUES ('market_bars_v2', '{tf}', {month}, {}, now64(9))",
                src_cells.n
            ))
            .execute()
            .await
            .map_err(|e| ch_err("record chunk", e))?;
        report.rows_copied += src_cells.n;
        report.chunks_copied += 1;
        info!(timeframe = %c.timeframe, month, cells = src_cells.n, "canonical chunk copied");
    }
    Ok(report)
}
