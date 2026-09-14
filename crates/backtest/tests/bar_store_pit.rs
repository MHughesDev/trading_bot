//! The single PIT bar reader/writer against a live ClickHouse (SPEC §1.2–§1.3;
//! INV-01, INV-02, INV-06; AT-02, AT-06).
//!
//! Gated on `BACKTEST_E2E_CLICKHOUSE_URL`; each test uses a throwaway database.
//!
//! ```bash
//! BACKTEST_E2E_CLICKHOUSE_URL=http://trading:trading@localhost:8123 \
//!   cargo test -j 2 -p backtest --test bar_store_pit -- --test-threads=1 --nocapture
//! ```

use std::sync::{Arc, OnceLock};

use backtest::store::{BarStore, CollectedBar};
use chrono::{DateTime, Duration, TimeZone, Utc};
use dataplane::quality::QualityFlags;
use domain::payloads::bar::Timeframe;
use rust_decimal::Decimal;
use storage::identity::{MemoryIdentity, SourceInfo};

fn identity() -> Arc<MemoryIdentity> {
    static ID: OnceLock<Arc<MemoryIdentity>> = OnceLock::new();
    ID.get_or_init(|| {
        let id = Arc::new(MemoryIdentity::new());
        id.add_source("rest", SourceInfo { source_id: 2, declared_vendor_lag: std::time::Duration::from_secs(60), live: false });
        id.add_source("ws", SourceInfo { source_id: 9, declared_vendor_lag: std::time::Duration::ZERO, live: true });
        storage::identity::install(id.clone());
        id
    })
    .clone()
}

async fn scratch(name: &str) -> Option<String> {
    let base = std::env::var("BACKTEST_E2E_CLICKHOUSE_URL").ok()?;
    let admin = storage::clickhouse::connect(&base);
    admin.query(&format!("DROP DATABASE IF EXISTS {name}")).execute().await.unwrap();
    admin.query(&format!("CREATE DATABASE {name}")).execute().await.unwrap();
    let (scheme, rest) = base.split_once("://").unwrap();
    let host = rest.split('/').next().unwrap();
    let url = format!("{scheme}://{host}/{name}");
    storage::clickhouse::migrate::run_migrations(&url).await.unwrap();
    Some(url)
}

fn bars(close_start: DateTime<Utc>, n: i64, price: impl Fn(i64) -> i64) -> Vec<CollectedBar> {
    (0..n)
        .map(|i| {
            let p = price(i).to_string();
            CollectedBar {
                available_time: close_start + Duration::minutes(i),
                sequence: i as u64,
                open: p.clone(),
                high: p.clone(),
                low: p.clone(),
                close: p,
                volume: "1".into(),
                trade_count: 1,
            }
        })
        .collect()
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 3, 3, 12, 1, 0).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn backfilled_history_is_flagged_and_invisible_before_its_sentinel() {
    let Some(url) = scratch("pit_backfill").await else { return };
    let id = identity();
    let w = BarStore::connect(&url);
    let r = w.write_collected("BF-USD", "coinbase", "rest", Timeframe::Minutes1, &bars(t0(), 10, |i| 100 + i)).await.unwrap();
    assert_eq!(r.inserted, 10);
    id.sync_dims(&url).await.unwrap();

    let loaded = BarStore::connect(&url).load_bars("BF-USD", Timeframe::Minutes1, t0() - Duration::minutes(1), t0() + Duration::hours(1)).await.unwrap();
    assert_eq!(loaded.len(), 10);
    for b in &loaded {
        assert!(b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(b.knowledge_ns - b.ts_ns, 60_000_000_000, "knowledge = close + declared lag");
        assert_eq!((b.ts_ns - b.open_ns), 60_000_000_000, "stored event_time is the bar open");
    }
    // As of 30s after the first bar closed, its sentinel knowledge time (close+60s)
    // has not arrived: the bar is invisible.
    let early = BarStore::connect(&url)
        .as_of(t0() + Duration::seconds(30))
        .load_bars("BF-USD", Timeframe::Minutes1, t0() - Duration::minutes(1), t0() + Duration::hours(1))
        .await
        .unwrap();
    assert!(early.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_recollection_writes_nothing_and_a_change_is_a_revision() {
    let Some(url) = scratch("pit_restate").await else { return };
    let id = identity();
    let w = BarStore::connect(&url);
    w.write_collected("RS-USD", "coinbase", "rest", Timeframe::Minutes1, &bars(t0(), 5, |_| 100)).await.unwrap();
    let again = w.write_collected("RS-USD", "coinbase", "rest", Timeframe::Minutes1, &bars(t0(), 5, |_| 100)).await.unwrap();
    assert_eq!((again.inserted, again.unchanged, again.restated), (0, 5, 0), "gap fills re-read ranges; that must be free");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let before_restatement = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let restated = w.write_collected("RS-USD", "coinbase", "rest", Timeframe::Minutes1, &bars(t0(), 5, |i| if i == 2 { 101 } else { 100 })).await.unwrap();
    assert_eq!((restated.unchanged, restated.restated), (4, 1));
    id.sync_dims(&url).await.unwrap();

    // The stored cell resolves by knowledge time. Knowledge of a REST backfill is
    // close + lag, so both versions share it; the later revision wins on
    // revision_seq, and it is flagged.
    let now = BarStore::connect(&url).load_bars("RS-USD", Timeframe::Minutes1, t0() - Duration::minutes(1), t0() + Duration::hours(1)).await.unwrap();
    assert_eq!(now.len(), 5, "a restatement is a new version, not a new bar");
    assert_eq!(now[2].close, Decimal::from(101));
    assert!(now[2].quality_flags.contains(QualityFlags::VENDOR_REVISED));
    let _ = before_restatement;
}

/// AT-06 shape for bars: a live bar revised later is seen as the original by an
/// as-of before the revision was known, and as the revision afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn live_revision_is_invisible_to_an_earlier_as_of() {
    let Some(url) = scratch("pit_live_rev").await else { return };
    let id = identity();
    let w = BarStore::connect(&url);
    let close = Utc::now() - Duration::seconds(5);
    let close = close - Duration::nanoseconds(close.timestamp_nanos_opt().unwrap() % 60_000_000_000);
    w.write_collected("LV-USD", "coinbase", "ws", Timeframe::Minutes1, &bars(close, 1, |_| 200)).await.unwrap();
    let after_first = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    w.write_collected("LV-USD", "coinbase", "ws", Timeframe::Minutes1, &bars(close, 1, |_| 201)).await.unwrap();
    id.sync_dims(&url).await.unwrap();

    let range = (close - Duration::minutes(2), close + Duration::minutes(2));
    let first = BarStore::connect(&url).as_of(after_first).load_bars("LV-USD", Timeframe::Minutes1, range.0, range.1).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].close, Decimal::from(200), "the revision was not yet known");
    assert!(!first[0].quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME), "observed live");
    let latest = BarStore::connect(&url).load_bars("LV-USD", Timeframe::Minutes1, range.0, range.1).await.unwrap();
    assert_eq!(latest[0].close, Decimal::from(201));
}

#[tokio::test(flavor = "multi_thread")]
async fn venues_are_never_blended_and_timeframes_never_collide() {
    let Some(url) = scratch("pit_venues").await else { return };
    let id = identity();
    let w = BarStore::connect(&url);
    // Registration order fixes MemoryIdentity venue ids: coinbase first ⇒ tier 1.
    w.write_collected("VX-USD", "coinbase", "rest", Timeframe::Minutes1, &bars(t0(), 3, |_| 10)).await.unwrap();
    w.write_collected("VX-USD", "kraken", "rest", Timeframe::Minutes1, &bars(t0(), 3, |_| 99)).await.unwrap();
    // A 1h bar closing on a 1m close does not destroy it.
    w.write_collected("VX-USD", "coinbase", "rest", Timeframe::Hours1, &[CollectedBar {
        available_time: Utc.with_ymd_and_hms(2025, 3, 3, 13, 0, 0).unwrap(),
        sequence: 0,
        open: "7".into(), high: "7".into(), low: "7".into(), close: "7".into(), volume: "1".into(), trade_count: 1,
    }]).await.unwrap();
    id.sync_dims(&url).await.unwrap();

    let range = (t0() - Duration::minutes(1), t0() + Duration::hours(2));
    let default_venue = BarStore::connect(&url).load_bars("VX-USD", Timeframe::Minutes1, range.0, range.1).await.unwrap();
    assert!(default_venue.iter().all(|b| b.close == Decimal::from(10)), "best-quality venue only");
    let kraken = BarStore::connect(&url).venue("kraken").load_bars("VX-USD", Timeframe::Minutes1, range.0, range.1).await.unwrap();
    assert!(kraken.iter().all(|b| b.close == Decimal::from(99)));
    let hourly = BarStore::connect(&url).load_bars("VX-USD", Timeframe::Hours1, range.0, range.1).await.unwrap();
    assert_eq!(hourly.len(), 1);
    assert_eq!(BarStore::connect(&url).load_bars("VX-USD", Timeframe::Minutes1, range.0, range.1).await.unwrap().len(), 3);
}

/// AT-02: a PIT read is within 1.2× of a non-PIT scan over the same rows. The
/// comparison scan lives only here, in a test, to prove the reader has no excuse
/// for a fast path.
#[tokio::test(flavor = "multi_thread")]
async fn pit_read_is_within_1_2x_of_a_raw_scan() {
    let Some(url) = scratch("pit_perf").await else { return };
    let id = identity();
    let w = BarStore::connect(&url);
    let start = Utc.with_ymd_and_hms(2024, 1, 1, 0, 1, 0).unwrap();
    for chunk in 0..20 {
        w.write_collected("PF-USD", "coinbase", "rest", Timeframe::Minutes1, &bars(start + Duration::minutes(5_000 * chunk), 5_000, |i| 100 + i % 7)).await.unwrap();
    }
    id.sync_dims(&url).await.unwrap();
    let ch = storage::clickhouse::connect(&url);
    ch.query("OPTIMIZE TABLE market_bar FINAL").execute().await.unwrap();
    let end = start + Duration::minutes(100_000);

    let mut pit = Vec::new();
    let mut raw = Vec::new();
    for _ in 0..7 {
        let t = std::time::Instant::now();
        let n = BarStore::connect(&url).load_bars("PF-USD", Timeframe::Minutes1, start, end + Duration::minutes(1)).await.unwrap().len();
        pit.push(t.elapsed());
        assert_eq!(n, 100_000);

        #[derive(clickhouse::Row, serde::Deserialize)]
        #[allow(dead_code)]
        struct Raw { open_ns: i64, knowledge_ns: i64, open: String, high: String, low: String, close: String, volume: String, trade_count: u64, quality_flags: u32 }
        let t = std::time::Instant::now();
        let rows: Vec<Raw> = ch
            // The equivalent non-PIT scan: the same bars and columns, no knowledge-time
            // filter, no restatement resolution, no symbol resolution.
            .query("SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS open_ns, toInt64(toUnixTimestamp64Nano(knowledge_time)) AS knowledge_ns, toString(open) AS open, toString(high) AS high, toString(low) AS low, toString(close) AS close, toString(volume) AS volume, toUInt64(ifNull(trade_count, 0)) AS trade_count, quality_flags FROM market_bar WHERE period_secs = 60 ORDER BY event_time")
            .fetch_all()
            .await
            .unwrap();
        // Same client-side work as the reader: decimals are parsed, never floats.
        let parsed: Vec<[Decimal; 5]> = rows
            .iter()
            .map(|r| [r.open.parse().unwrap(), r.high.parse().unwrap(), r.low.parse().unwrap(), r.close.parse().unwrap(), r.volume.parse().unwrap()])
            .collect();
        raw.push(t.elapsed());
        assert_eq!(parsed.len(), 100_000);
    }
    pit.sort();
    raw.sort();
    let (p, r) = (pit[3].as_secs_f64(), raw[3].as_secs_f64());
    println!("median pit {p:.3}s raw {r:.3}s ratio {:.2}", p / r);
    assert!(p <= r * 1.2, "PIT read {p:.3}s exceeds 1.2× raw {r:.3}s");
}
