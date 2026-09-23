//! One-time migration of `market_bars_v2` into the canonical `market_bar`
//! (clickhouse/07_canonical_market.sql).
//!
//! What changes on the way across, per row:
//! * identity: `(venue name, symbol)` → surrogate `(instrument_id, venue_id)`;
//! * convention: `event_time` becomes the bar OPEN (v2's `bar_open_time`);
//! * precision: Decimal(38,10) → Decimal(38,18);
//! * knowledge: decided **per row, from that row's own receipt** — see below.
//!
//! # The knowledge rule, and the axis it turns on
//!
//! A row whose ingest stamp falls between its bar's close and its vendor's
//! publication plus a grace window was watched arrive: its knowledge time is that
//! receipt, unflagged, and it is forward evidence under Set P D-P3. Every other row
//! gets `close + declared vendor lag` and `BACKFILLED_KNOWLEDGE_TIME` (CLAUDE.md §6,
//! ADR-P2-24). Nothing is fabricated in either direction.
//!
//! This module previously required `source.kind = 'live_stream'` as well, which is
//! the transport, not the receipt. That is wrong twice over: it marks every forward
//! REST poll derived, so a forward collector supplies Gate 15 with nothing; and it
//! replaces a real receipt with a sentinel that can land *later* than the truth.
//! Track 3 shipped and then corrected exactly this bug in `dataplane::source`
//! (commit 3596384); this is the same correction on the copy path. One source
//! legitimately supplies both legs, so the question has to be asked of the row.
//!
//! The classification is [`dataplane::bar::classify_receipt`] — the same function
//! the writer uses — expressed here in SQL so 158,259 rows do not cross the process
//! boundary one at a time. `classification_sql_matches_the_pure_kernel` in
//! `tests/canonical_migration.rs` pins the two together.
//!
//! Chunked by (timeframe, month), recorded in `canonical_migration_chunk`, verified
//! per chunk by distinct-cell counts, and skipped once done.

use std::collections::BTreeSet;

use clickhouse::Row;
use serde::Deserialize;
use sqlx::PgPool;
use tracing::{error, info, warn};

use super::ChError;
use crate::identity::{IdentityResolver, PgIdentityService};
use dataplane::identity::AssetClass;

const BACKFILLED: u32 = dataplane::quality::QualityFlags::BACKFILLED_KNOWLEDGE_TIME.0;
/// How much later than its vendor's publication a receipt may land and still read as
/// one. Mirrors [`dataplane::bar::RECEIPT_GRACE`]; pinned by
/// `the_grace_window_matches_the_pure_kernel`.
const RECEIPT_GRACE_SECS: i64 = 300;
/// `unknown_legacy` in dataplane.source.
const UNKNOWN_SOURCE_ID: i32 = 6;
const UNKNOWN_SOURCE_LAG_MS: i64 = 60_000;

/// Which rule stamped a chunk's knowledge times.
///
/// 1 — observed-versus-derived from the source's transport. Wrong: see the module
///     docs. Chunks recorded under it hold knowledge times nobody should trust.
/// 2 — from the row's own receipt. The rule ADR-P2-24 and `dataplane::source`
///     describe.
const PROVENANCE_RULE: u8 = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanonicalReport {
    pub identities: usize,
    pub chunks_copied: usize,
    pub chunks_skipped: usize,
    pub rows_copied: u64,
    pub backfilled_rows: u64,
    /// The split, summed over every chunk this run copied. These three are the
    /// numbers the repair exists to produce; `observed_rows` in particular is the
    /// platform's stock of Gate-15-admissible forward evidence (D-P3).
    pub observed_rows: u64,
    pub history_pull_rows: u64,
    pub impossible_rows: u64,
    /// Chunks already on disk that were copied under provenance rule 1. Their
    /// knowledge times were decided by transport and are not trustworthy. They are
    /// **reported, not silently rewritten**: `knowledge_time` is in `market_bar`'s
    /// sorting key, so a corrected copy would land beside the wrong one as a second
    /// version rather than replacing it, and deleting rows to fix provenance is not
    /// a repair. Rebuilding those partitions is an operator action.
    pub stale_rule_chunks: Vec<(String, u32)>,
}

impl CanonicalReport {
    /// Rows that can honestly be called observed, over the chunks this run copied.
    #[must_use]
    pub const fn defensibly_observed_rows(&self) -> u64 {
        self.observed_rows
    }
}

/// The receipt classification, in SQL, over the v2 row alias `v` and the source
/// alias `src`.
///
/// Exactly [`dataplane::bar::classify_receipt`]:
/// * `ingested_time < close`            → impossible; never an observation;
/// * `ingested_time <= close + lag + g` → a receipt;
/// * otherwise                          → a history pull.
///
/// v2's `event_time` is the bar CLOSE, so `close` is `v.event_time` here even though
/// the canonical table's `event_time` is the OPEN.
fn receipt_class_sql() -> String {
    format!(
        "multiIf(v.ingested_time < v.event_time, 2, \
                 v.ingested_time <= v.event_time + toIntervalMillisecond({lag}) + INTERVAL {RECEIPT_GRACE_SECS} SECOND, 0, \
                 1)",
        lag = source_lag_ms_sql(),
    )
}

/// The row's declared vendor lag in milliseconds, with `unknown_legacy`'s 60 s
/// standing in for a source the map does not know (ADR-P2-24).
fn source_lag_ms_sql() -> String {
    format!("if(src.source_id = 0, {UNKNOWN_SOURCE_LAG_MS}, src.lag_ms)")
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
            provenance_rule: u8,
        }
        let rows: Vec<Done> = client
            .query(
                "SELECT timeframe, month, provenance_rule FROM canonical_migration_chunk FINAL \
                  WHERE source_table = 'market_bars_v2'",
            )
            .fetch_all()
            .await
            .map_err(|e| ch_err("done chunks", e))?;
        // A chunk copied under rule 1 was stamped from the transport, not the row's
        // receipt. It is recorded here so the boot says so out loud rather than
        // skipping it as "already done", but it is *not* re-copied: see
        // `CanonicalReport::stale_rule_chunks`.
        for d in &rows {
            if d.provenance_rule < PROVENANCE_RULE {
                report.stale_rule_chunks.push((d.timeframe.clone(), d.month));
            }
        }
        rows.into_iter().map(|d| (d.timeframe, d.month)).collect()
    };

    for c in chunks {
        if done.contains(&(c.timeframe.clone(), c.month)) {
            report.chunks_skipped += 1;
            continue;
        }
        let tf = c.timeframe.replace('\'', "''");
        let month = c.month;
        // The receipt axis, per row. `src.live` is deliberately unused here: the
        // transport does not decide provenance (see the module docs).
        let class = receipt_class_sql();
        let lag_ms = source_lag_ms_sql();
        let sentinel = format!("v.event_time + toIntervalMillisecond({lag_ms})");
        let insert = format!(
            "INSERT INTO market_bar
             SELECT s.instrument_id, vd.venue_id,
                    toUInt32(dateDiff('second', v.bar_open_time, v.event_time)) AS period_secs,
                    v.bar_open_time AS event_time,
                    CAST(NULL AS Nullable(DateTime64(9, 'UTC'))) AS venue_ts,
                    v.ingested_time AS ingest_time,
                    if({class} = 0, greatest(v.ingested_time, {sentinel}), {sentinel}) AS knowledge_time,
                    CAST(v.open AS Decimal(38, 18)), CAST(v.high AS Decimal(38, 18)), CAST(v.low AS Decimal(38, 18)),
                    CAST(v.close AS Decimal(38, 18)), CAST(v.volume AS Decimal(38, 18)),
                    CAST(toUInt32(least(v.trade_count, 4294967295)) AS Nullable(UInt32)) AS trade_count,
                    CAST(NULL AS Nullable(Decimal(38, 18))), CAST(NULL AS Nullable(Decimal(38, 18))), CAST(NULL AS Nullable(Decimal(38, 18))),
                    CAST(NULL AS Nullable(Float64)), CAST(NULL AS Nullable(UInt32)),
                    if({class} = 0, toUInt32(0), toUInt32({BACKFILLED})) AS quality_flags,
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

        // The split, counted from the source rows under the same classification the
        // copy used. This is the number the whole exercise exists to produce, and it
        // is written down rather than logged: an operator asking "how much of this
        // history can honestly be called observed" should not have to re-derive it.
        #[derive(Row, Deserialize)]
        struct Split {
            observed: u64,
            history: u64,
            impossible: u64,
        }
        let split: Split = client
            .query(&format!(
                "SELECT countIf({class} = 0) AS observed,
                        countIf({class} = 1) AS history,
                        countIf({class} = 2) AS impossible
                   FROM (SELECT * FROM market_bars_v2 FINAL WHERE timeframe = '{tf}' AND toYYYYMM(event_time) = {month}) AS v
                   LEFT JOIN (SELECT * FROM values('name String, source_id Int32, lag_ms Int64, live UInt8', {source_values})) AS src
                          ON src.name = v.source"
            ))
            .fetch_one()
            .await
            .map_err(|e| ch_err("count provenance split", e))?;
        client
            .query(&format!(
                "INSERT INTO knowledge_time_repair
                   (source_table, timeframe, month, observed_rows, history_pull_rows, impossible_rows, provenance_rule, repaired_at)
                 VALUES ('market_bars_v2', '{tf}', {month}, {}, {}, {}, {PROVENANCE_RULE}, now64(9))",
                split.observed, split.history, split.impossible
            ))
            .execute()
            .await
            .map_err(|e| ch_err("record repair split", e))?;
        report.observed_rows += split.observed;
        report.history_pull_rows += split.history;
        report.impossible_rows += split.impossible;

        client
            .query(&format!(
                "INSERT INTO canonical_migration_chunk (source_table, timeframe, month, rows_copied, copied_at, provenance_rule)
                 VALUES ('market_bars_v2', '{tf}', {month}, {}, now64(9), {PROVENANCE_RULE})",
                src_cells.n
            ))
            .execute()
            .await
            .map_err(|e| ch_err("record chunk", e))?;
        report.rows_copied += src_cells.n;
        report.chunks_copied += 1;
        info!(
            timeframe = %c.timeframe,
            month,
            cells = src_cells.n,
            observed = split.observed,
            history_pull = split.history,
            impossible = split.impossible,
            "canonical chunk copied"
        );
        if split.impossible > 0 {
            warn!(
                timeframe = %c.timeframe,
                month,
                rows = split.impossible,
                "rows carry an ingest stamp earlier than their own bar close; they were given the \
                 sentinel and the backfill flag, never an observation, but a clock or a parser is wrong"
            );
        }
    }

    if !report.stale_rule_chunks.is_empty() {
        error!(
            chunks = report.stale_rule_chunks.len(),
            "canonical chunks on disk were copied under provenance rule 1 (transport, not receipt). \
             Their knowledge_time values are not trustworthy: every forward poll in them is marked \
             derived and every sentinel may post-date the truth. knowledge_time is in market_bar's \
             sorting key, so these cannot be corrected in place — the partitions must be rebuilt from \
             market_bars_v2, which is intact. Nothing has been deleted."
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SQL and the pure kernel must use the same grace window, or the repair of
    /// existing rows and the stamping of new ones disagree about what a receipt is —
    /// and the disagreement would only show up as rows that are observed in one path
    /// and derived in the other.
    #[test]
    fn the_grace_window_matches_the_pure_kernel() {
        assert_eq!(RECEIPT_GRACE_SECS, dataplane::bar::RECEIPT_GRACE.num_seconds());
    }

    /// The regression this module was fixed for. Provenance must not be keyed on the
    /// source's transport, so the copy's knowledge expression must not consult
    /// `src.live` at all. Naming the column here rather than in a comment means the
    /// old rule cannot be reintroduced quietly.
    #[test]
    fn the_copy_does_not_decide_provenance_from_the_transport() {
        let class = receipt_class_sql();
        assert!(!class.contains("src.live"), "transport is not provenance: {class}");
        assert!(!class.contains("kind"), "{class}");
        assert!(class.contains("v.ingested_time"), "the axis is the row's receipt: {class}");
    }

    /// The three class codes the copy branches on, spelled out so a reordering of
    /// `multiIf` cannot silently swap "observed" and "history pull".
    #[test]
    fn the_class_codes_are_observed_zero_history_one_impossible_two() {
        let class = receipt_class_sql();
        // Impossible is tested first and yields 2; the receipt window yields 0; the
        // fallthrough is 1.
        let impossible = class.find("v.ingested_time < v.event_time, 2").expect("impossible arm");
        let receipt = class.find("SECOND, 0").expect("receipt arm");
        assert!(impossible < receipt, "an impossible receipt must be caught before the window test");
        assert!(class.trim_end().ends_with("1)"), "the fallthrough is a history pull: {class}");
    }

    /// A source the map does not know falls back to 60 s and `unknown_legacy`, which
    /// is ADR-P2-24's answer for a row whose origin cannot be determined — never a
    /// zero lag, which would read as "observed live".
    #[test]
    fn an_unknown_source_falls_back_to_the_declared_legacy_lag() {
        assert_eq!(UNKNOWN_SOURCE_LAG_MS, 60_000);
        assert_eq!(UNKNOWN_SOURCE_ID, 6);
        assert!(source_lag_ms_sql().contains("60000"));
    }
}
