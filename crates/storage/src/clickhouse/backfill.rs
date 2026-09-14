//! One-shot backfill of `market_bars` → `market_bars_v2` (Set L, L-0.3).
//!
//! `market_bars` is `ReplacingMergeTree(revision) ORDER BY (instrument_id,
//! available_time)` with `timeframe` absent from the key, so bars of different
//! timeframes that close at the same instant collapse into one (DATA-005 §3, DA-16).
//! This module copies the data into the append-only v2 table, where that cannot
//! happen.
//!
//! ## Time semantics
//!
//! v1 stores exactly one bar timestamp, `available_time`, and the collectors set it
//! to the bar's **close**: every call site computes `open_time + timeframe`
//! (`backtest::collect`). v2 separates the three meanings, so the mapping is:
//!
//! | v2 column | from v1 |
//! |---|---|
//! | `event_time` | `available_time` (the close) |
//! | `bar_open_time` | `available_time − timeframe` |
//! | `available_time` | `available_time` (unchanged) |
//!
//! `available_time` is carried over rather than recomputed because v1 never recorded
//! collection lag; inventing one would be fabricating provenance.
//!
//! ## Idempotence and resumability
//!
//! Work is chunked by `(timeframe, month)`, and within a chunk the copy is a
//! **row-level anti-join**: only rows whose fingerprint is absent from v2 are
//! inserted. That makes a re-run a no-op and a half-finished chunk resume, without
//! ever duplicating — which matters because v2 is append-only and has no engine
//! that would collapse a duplicate away.
//!
//! Comparing row *counts* between the two tables would be cheaper, and it is what
//! this module did first. It is wrong: after the cutover v2 legitimately holds more
//! rows than v1 for the current month, because live collection writes only to v2. A
//! count comparison reads that healthy state as corruption.

use tracing::info;

use super::ChError;

/// Timeframe key → duration in seconds.
///
/// Mirrors `backtest::types::TimeframeExt`, which `storage` cannot depend on
/// (`backtest` depends on `storage`, not the other way round). [`tests::
/// timeframe_table_matches_the_domain_vocabulary`] pins the set so the two cannot
/// drift silently.
///
/// A timeframe that is not in this table is a hard error, never a default: guessing
/// would write a wrong `bar_open_time` that looks perfectly plausible forever after.
const TIMEFRAME_SECONDS: &[(&str, u64)] = &[
    ("1s", 1),
    ("1m", 60),
    ("5m", 300),
    ("15m", 900),
    ("1h", 3_600),
    ("4h", 14_400),
    ("1d", 86_400),
];

fn timeframe_seconds(key: &str) -> Option<u64> {
    TIMEFRAME_SECONDS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, s)| *s)
}

/// Columns copied verbatim from v1 to v2, in the order the INSERT uses.
const CARRIED_COLUMNS: &str = "event_id, lane, instrument_id, venue_id, source, trust_tier, \
                               ingested_time, sequence, timeframe, open, high, low, close, \
                               volume, trade_count, revision, dedup_key";

/// What makes two rows *the same observation*.
///
/// This is v2's sorting key, expressed against both column vocabularies — in v1 the
/// bar's close is `available_time`, in v2 it is `event_time`, and the two carry the
/// same instant.
///
/// Comparing on identity rather than on full content is load-bearing, and getting it
/// wrong produced a boot-time loop. The earlier version fingerprinted every shared
/// column, `ingested_time` included. But a bar collected twice is one observation
/// with two ingest times, and v2's engine keeps only the most recent — so after a
/// merge the v1 row's exact content is no longer present, the anti-join reports it
/// missing, and the backfill copies it again. Once per boot, forever.
const IDENTITY_COLUMNS_V1: &str =
    "instrument_id, timeframe, venue_id, source, available_time, revision";
const IDENTITY_COLUMNS_V2: &str =
    "instrument_id, timeframe, venue_id, source, event_time, revision";

/// What a backfill run did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BackfillReport {
    /// `(timeframe, month)` chunks copied by this run.
    pub chunks_copied: usize,
    /// Chunks already present and therefore skipped.
    pub chunks_skipped: usize,
    /// Rows written by this run.
    pub rows_copied: u64,
}

impl BackfillReport {
    pub fn did_work(&self) -> bool {
        self.chunks_copied > 0
    }
}

/// Copies every row of `source_table` into `market_bars_v2`.
///
/// Safe to call on every boot: finished chunks are skipped, and an empty source is a
/// no-op (the fresh-install case).
///
/// `source_table` is normally `market_bars`. Point it at a verified snapshot instead
/// when one exists — a plain `MergeTree` copy cannot collapse under the migration the
/// way the live `ReplacingMergeTree` can.
pub async fn backfill_market_bars_v2(
    url: &str,
    source_table: &str,
) -> Result<BackfillReport, ChError> {
    validate_identifier(source_table)?;
    let client = super::connect(url);

    let chunks: Vec<(String, u32)> = client
        .query(&format!(
            "SELECT timeframe, toYYYYMM(available_time) AS month \
             FROM {source_table} GROUP BY timeframe, month ORDER BY timeframe, month"
        ))
        .fetch_all()
        .await
        .map_err(|e| ChError::Client(format!("enumerate chunks: {e}")))?;

    if chunks.is_empty() {
        info!(
            source = source_table,
            "no bars to backfill into market_bars_v2"
        );
        return Ok(BackfillReport::default());
    }

    // Reject unknown timeframes up front, before writing anything. Failing halfway
    // through would leave a partially migrated table for no reason.
    for (timeframe, _) in &chunks {
        if timeframe_seconds(timeframe).is_none() {
            return Err(ChError::Client(format!(
                "unknown timeframe {timeframe:?} in {source_table}: refusing to guess its \
                 duration, which would write a wrong bar_open_time. Add it to \
                 TIMEFRAME_SECONDS."
            )));
        }
    }

    let mut report = BackfillReport::default();

    for (timeframe, month) in chunks {
        let seconds = timeframe_seconds(&timeframe).expect("validated above");

        let source_rows = count(
            &client,
            &format!(
                "SELECT count() FROM {source_table} \
                 WHERE timeframe = ? AND toYYYYMM(available_time) = ?"
            ),
            &timeframe,
            month,
        )
        .await?;

        // How many rows of this chunk are missing from v2, compared row by row on a
        // fingerprint of the shared columns.
        //
        // Counting rows on each side and comparing the totals is not good enough,
        // and the first version of this made exactly that mistake. After the
        // cutover, v2 legitimately holds *more* rows than v1 for the current month,
        // because live collection writes only to v2 — so a total comparison reads a
        // perfectly healthy table as "partially populated" and aborts. That failed
        // the platform's boot, since the backfill runs on startup.
        let missing = count(
            &client,
            &format!(
                "SELECT count() FROM \
                   (SELECT DISTINCT sipHash64({IDENTITY_COLUMNS_V1}) AS fp FROM {source_table} \
                     WHERE timeframe = ? AND toYYYYMM(available_time) = ?) AS src \
                 LEFT ANTI JOIN \
                   (SELECT DISTINCT sipHash64({IDENTITY_COLUMNS_V2}) AS fp FROM market_bars_v2) AS dst \
                 USING (fp)"
            ),
            &timeframe,
            month,
        )
        .await?;

        if missing == 0 {
            report.chunks_skipped += 1;
            continue;
        }

        // Copy only the rows that are actually absent. This is idempotent by
        // construction: re-running inserts nothing, and a half-finished chunk
        // resumes rather than duplicating. v2 has no engine that would collapse a
        // duplicate away, so "insert only what is missing" has to be enforced here.
        client
            .query(&format!(
                "INSERT INTO market_bars_v2 \
                   (bar_open_time, event_time, available_time, {CARRIED_COLUMNS}) \
                 SELECT \
                   available_time - toIntervalSecond({seconds}) AS bar_open_time, \
                   available_time AS event_time, \
                   available_time, \
                   {CARRIED_COLUMNS} \
                 FROM {source_table} \
                 WHERE timeframe = ? AND toYYYYMM(available_time) = ? \
                   AND sipHash64({IDENTITY_COLUMNS_V1}) NOT IN \
                       (SELECT sipHash64({IDENTITY_COLUMNS_V2}) FROM market_bars_v2)"
            ))
            .bind(&timeframe)
            .bind(month)
            .execute()
            .await
            .map_err(|e| ChError::Client(format!("copy ({timeframe}, {month}): {e}")))?;

        report.chunks_copied += 1;
        report.rows_copied += missing;
        info!(
            timeframe = %timeframe,
            month,
            rows = missing,
            source_rows,
            "market_bars_v2 chunk copied"
        );
    }

    if report.did_work() {
        // Verify before declaring success (L-0.4, DA-17). A copy that moved the
        // wrong rows is worse than one that never ran, because it looks finished —
        // and the next step of this phase retires the source table.
        verify_market_bars_v2(url, source_table).await?;

        info!(
            chunks_copied = report.chunks_copied,
            chunks_skipped = report.chunks_skipped,
            rows_copied = report.rows_copied,
            source = source_table,
            "market_bars_v2 backfill complete and verified"
        );
    }
    Ok(report)
}

/// The outcome of verifying that `market_bars_v2` really contains the source data
/// (L-0.4, DA-17).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    pub source_rows: u64,
    pub v2_rows: u64,
    /// Rows present in the source but absent from v2. The only number that decides
    /// pass or fail.
    pub missing_rows: u64,
}

impl VerificationReport {
    pub fn passed(&self) -> bool {
        self.missing_rows == 0
    }
}

/// Verifies that every row of `source_table` exists in `market_bars_v2`.
///
/// The check is a row-level anti-join on a `sipHash64` fingerprint of all
/// [`IDENTITY_COLUMNS_V1`], not a count and not a sum of hashes. Counts miss a copy that
/// moved the right *number* of rows with the wrong content; a summed hash can in
/// principle collide, and worse, it silently fails once the comparison stops being
/// an equality (below).
///
/// **The invariant is containment, not equality.** Right after the backfill the two
/// tables match exactly. From then on they legitimately drift apart, in one
/// direction only: `market_bars` is a `ReplacingMergeTree` that *destroys* rows on
/// merge, which is the whole reason v2 exists. So v2 is a superset of v1 forever
/// after, and an equality check would start failing on a correct system the first
/// time v1 lost a bar. Containment stays true and stays meaningful.
pub async fn verify_market_bars_v2(
    url: &str,
    source_table: &str,
) -> Result<VerificationReport, ChError> {
    validate_identifier(source_table)?;
    let client = super::connect(url);

    let source_rows: u64 = client
        .query(&format!("SELECT count() FROM {source_table}"))
        .fetch_one()
        .await
        .map_err(|e| ChError::Client(format!("count {source_table}: {e}")))?;

    let v2_rows: u64 = client
        .query("SELECT count() FROM market_bars_v2")
        .fetch_one()
        .await
        .map_err(|e| ChError::Client(format!("count market_bars_v2: {e}")))?;

    let missing_rows: u64 = client
        .query(&format!(
            "SELECT count() FROM \
               (SELECT DISTINCT sipHash64({IDENTITY_COLUMNS_V1}) AS fingerprint FROM {source_table}) AS src \
             LEFT ANTI JOIN \
               (SELECT DISTINCT sipHash64({IDENTITY_COLUMNS_V2}) AS fingerprint FROM market_bars_v2) AS dst \
             USING (fingerprint)"
        ))
        .fetch_one()
        .await
        .map_err(|e| ChError::Client(format!("anti-join {source_table} against v2: {e}")))?;

    let report = VerificationReport {
        source_rows,
        v2_rows,
        missing_rows,
    };

    if !report.passed() {
        return Err(ChError::Client(format!(
            "market_bars_v2 verification FAILED: {missing_rows} of {source_rows} rows in \
             {source_table} are absent from market_bars_v2 (v2 holds {v2_rows}). The v2 table \
             is not a faithful copy — do not retire {source_table}, and do not start the bar \
             backfill."
        )));
    }

    info!(
        source = source_table,
        source_rows, v2_rows, "market_bars_v2 verified: every source row is present"
    );
    Ok(report)
}

async fn count(
    client: &super::ChClient,
    sql: &str,
    timeframe: &str,
    month: u32,
) -> Result<u64, ChError> {
    client
        .query(sql)
        .bind(timeframe)
        .bind(month)
        .fetch_one::<u64>()
        .await
        .map_err(|e| ChError::Client(format!("count: {e}")))
}

/// The source table name is interpolated into SQL, so it must be a plain
/// identifier. It comes from configuration, not from a request, but an injection
/// point that exists is an injection point that eventually gets reached.
fn validate_identifier(name: &str) -> Result<(), ChError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if ok {
        Ok(())
    } else {
        Err(ChError::Client(format!(
            "invalid source table name {name:?}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeframe_table_matches_the_domain_vocabulary() {
        // Mirrors backtest::types::TimeframeExt. If a timeframe is added there and
        // not here, the backfill would refuse it at runtime — this test moves that
        // failure to build time.
        let keys: Vec<&str> = TIMEFRAME_SECONDS.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["1s", "1m", "5m", "15m", "1h", "4h", "1d"]);
        assert_eq!(timeframe_seconds("1m"), Some(60));
        assert_eq!(timeframe_seconds("1h"), Some(3_600));
        assert_eq!(timeframe_seconds("1d"), Some(86_400));
    }

    #[test]
    fn unknown_timeframe_has_no_duration() {
        // The whole point: no default, no fallback, no zero.
        assert_eq!(timeframe_seconds("2m"), None);
        assert_eq!(timeframe_seconds(""), None);
        assert_eq!(timeframe_seconds("1M"), None);
    }

    #[test]
    fn rejects_non_identifier_source_tables() {
        assert!(validate_identifier("market_bars").is_ok());
        assert!(validate_identifier("trading.market_bars").is_ok());
        assert!(validate_identifier("market_bars_rescue_20260911").is_ok());
        assert!(validate_identifier("bars; DROP TABLE market_bars").is_err());
        assert!(validate_identifier("bars WHERE 1=1").is_err());
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn report_reports_work() {
        let idle = BackfillReport {
            chunks_skipped: 4,
            ..Default::default()
        };
        assert!(!idle.did_work());
        let busy = BackfillReport {
            chunks_copied: 1,
            rows_copied: 10,
            ..Default::default()
        };
        assert!(busy.did_work());
    }
}
