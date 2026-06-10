//! Raw event archive partition-path logic.
//!
//! # Partition contract (P0-T11, implemented in Phase 1)
//!
//! The ground-truth raw normalized event archive is stored in object storage
//! (S3-compatible) as Parquet files.  The partition layout is:
//!
//! ```text
//! s3://bucket/events/{lane}/venue={venue_id}/instrument={instrument_id}/date={date}/
//! ```
//!
//! Examples:
//! ```text
//! s3://bucket/events/market_bars/venue=coinbase/instrument=BTC-USDT/date=2026-06-08/
//! s3://bucket/events/market_trades/venue=alpaca/instrument=AAPL/date=2026-06-08/
//! ```
//!
//! ## Invariants
//!
//! - **Append-only and immutable.** No event is ever modified in place.  Late data
//!   writes a new revision event (new row) — it never replaces an existing file.
//! - **Written before any derivation.**  Raw events land here before bars are built,
//!   before features are computed, before any derived store is written.
//! - **Batching policy:** 10,000 events OR 100 ms elapsed, whichever comes first.
//!   Each batch flush creates one Parquet file in the partition.
//! - **Nightly compaction:** a compaction job merges the small batch files in each
//!   `lane/venue/instrument/date` partition into a single large file, maintaining
//!   efficient read performance for replay and analytics queries without holding open large files
//!   during live operation.
//!
//! This function signature is the contract Phase 1's writer must implement.

use chrono::NaiveDate;

/// Compute the object-storage key prefix for a partition.
///
/// The returned path is a prefix — append a filename (e.g. `{batch_id}.parquet`)
/// to get a complete object key.
///
/// # Arguments
/// - `lane` — canonical lane name, `/` replaced with `_` for path safety
///   (e.g. `"market.bars.1m"` → `"market_bars_1m"`).
/// - `venue_id` — e.g. `"coinbase"`, `"alpaca"`.
/// - `instrument_id` — e.g. `"BTC-USDT"`, `"AAPL"`.
/// - `date` — UTC date of the events in this partition.
pub fn partition_path(lane: &str, venue_id: &str, instrument_id: &str, date: NaiveDate) -> String {
    let lane_safe = lane.replace('.', "_");
    format!(
        "events/{}/venue={}/instrument={}/date={}/",
        lane_safe, venue_id, instrument_id, date
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn partition_path_format() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        let path = partition_path("market.bars.1m", "coinbase", "BTC-USDT", date);
        assert_eq!(
            path,
            "events/market_bars_1m/venue=coinbase/instrument=BTC-USDT/date=2026-06-08/"
        );
    }
}
