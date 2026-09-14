//! Run the `market_bars` → `market_bars_v2` backfill as a standalone operation
//! (Set L, L-0.3), without booting the whole platform.
//!
//! The platform runs this on boot, but a one-time migration of an existing database
//! is better done deliberately, with the report in front of you — and from a
//! *verified snapshot* rather than the live `ReplacingMergeTree` table, which can
//! collapse rows underneath a long-running copy.
//!
//! ```bash
//! CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//! BARS_V2_BACKFILL_SOURCE=market_bars_rescue_20260911 \
//!   cargo run -j 2 -p storage --example backfill_bars_v2
//! ```
//!
//! Idempotent: chunks already copied are skipped, so re-running is safe. It never
//! writes to the source table.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("CLICKHOUSE_URL")
        .map_err(|_| "set CLICKHOUSE_URL, e.g. http://user:pass@localhost:8123/trading")?;
    let source =
        std::env::var("BARS_V2_BACKFILL_SOURCE").unwrap_or_else(|_| "market_bars".to_string());

    println!("backfilling market_bars_v2 from {source}");

    let report = storage::clickhouse::backfill::backfill_market_bars_v2(&url, &source).await?;

    println!(
        "copied: {} rows across {} chunks, {} chunks already present",
        report.rows_copied, report.chunks_copied, report.chunks_skipped
    );
    if !report.did_work() {
        println!("(nothing to copy — the backfill had already completed)");
    }

    // Always verify, even when nothing was copied. Someone running this by hand is
    // asking "is v2 a faithful copy?", and the answer should not depend on whether
    // this particular invocation happened to do any work.
    let verified = storage::clickhouse::backfill::verify_market_bars_v2(&url, &source).await?;
    println!(
        "verified: {} source rows, {} rows in market_bars_v2, {} missing",
        verified.source_rows, verified.v2_rows, verified.missing_rows
    );
    println!("OK — every row of {source} is present in market_bars_v2");
    Ok(())
}
