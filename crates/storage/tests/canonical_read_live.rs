//! P-4.1 clause 1 on the live store, through the provenance read: one day of
//! Coinbase BTC-USD 1m bars resampled to 1h agrees with Coinbase's own 1h bars.
//!
//! **Read-only** against the database in `STORAGE_LIVE_READ_CLICKHOUSE_URL` (it
//! writes nothing), and gated on it because CI has no store holding venue data.
//! A skipped run is not evidence; the phase file records the run that was made.

use std::time::Duration;

use chrono::{TimeZone, Utc};
use dataplane::qc::Grade;
use dataplane::resample::{resample, ResampleOptions, SourceProvenance};
use storage::clickhouse::canonical_read::canonical_bars_as_of;

#[tokio::test]
async fn one_minute_bars_resample_to_the_venues_own_hour() {
    let Ok(url) = std::env::var("STORAGE_LIVE_READ_CLICKHOUSE_URL") else {
        eprintln!("STORAGE_LIVE_READ_CLICKHOUSE_URL unset - skipping (not evidence)");
        return;
    };
    let from = Utc.with_ymd_and_hms(2026, 9, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 16, 0, 0, 0).unwrap();
    let now = Utc::now();
    let minutes = canonical_bars_as_of(&url, "BTC-USD", "coinbase", 60, from, to, now)
        .await
        .unwrap();
    let hours = canonical_bars_as_of(&url, "BTC-USD", "coinbase", 3600, from, to, now)
        .await
        .unwrap();
    assert!(minutes.len() > 1_400, "a backfilled day: {}", minutes.len());
    assert_eq!(hours.len(), 24);
    assert!(minutes.iter().all(|b| b.source_id > 0), "provenance is carried");

    let prov: Vec<SourceProvenance> = minutes
        .iter()
        .map(|b| b.source_id)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|source_id| SourceProvenance {
            source_id,
            grade: Grade::B,
            declared_lag: Duration::from_secs(60),
        })
        .collect();
    let derived = resample(&minutes, 3600, &prov, ResampleOptions::default()).unwrap();
    assert_eq!(derived.bars.len(), 24);
    let mut mismatched = Vec::new();
    for (d, v) in derived.bars.iter().zip(&hours) {
        assert_eq!(d.event_time, v.event_time);
        if d.complete && (d.open, d.high, d.low, d.close) != (v.open, v.high, v.low, v.close) {
            mismatched.push((d.event_time, d.open, v.open, d.close, v.close));
        }
    }
    assert!(mismatched.is_empty(), "derived hours disagree with the venue's: {mismatched:?}");

    // As of a moment before the backfill ran, none of it is visible.
    let before = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    assert!(
        canonical_bars_as_of(&url, "BTC-USD", "coinbase", 3600, from, to, before)
            .await
            .unwrap()
            .is_empty(),
        "bars for 2026-09-15 cannot be known on 2026-09-01"
    );
}
