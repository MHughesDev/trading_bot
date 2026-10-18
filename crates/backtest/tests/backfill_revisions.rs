//! Revisions are retained, and backfilled rows stay distinguishable from
//! observed ones — end to end, against a live ClickHouse
//! (Set P P-1.5, P-1.6; DA-16; ADR-P2-24; INV-01, INV-02).
//!
//! Gated on `BACKTEST_E2E_CLICKHOUSE_URL`; each test uses a throwaway database.
//!
//! ```bash
//! BACKTEST_E2E_CLICKHOUSE_URL=http://trading:trading@localhost:8123 \
//!   cargo test -j 2 -p backtest --test backfill_revisions -- --test-threads=1
//! ```
//!
//! Backfill is exactly where a second version of an already-stored bar arrives,
//! so it is where "revisions are retained" stops being a schema property and
//! becomes a behaviour that can be wrong. Two things are proved here that
//! nothing else in the suite proves together:
//!
//! 1. A revised bar produces a **second row**, and an `as_of` read before the
//!    revision was known returns the **original** value. Retaining a revision
//!    that no point-in-time read can reach is a PIT hole wearing a PIT store's
//!    clothes.
//! 2. The same bar written by a live collector and by a backfill yields two rows
//!    that differ **both** in `BACKFILLED_KNOWLEDGE_TIME` and in
//!    `knowledge_time`. Either signal alone survives somebody dropping the
//!    other; the test fails if either is dropped.

use std::sync::{Arc, OnceLock};

use backtest::backfill_sink::BarStoreSink;
use backtest::store::{BarStore, CollectedBar};
use chrono::{DateTime, Duration, TimeZone, Utc};
use dataplane::backfill::{BackfillBar, BarSink};
use dataplane::quality::QualityFlags;
use domain::payloads::bar::Timeframe;
use rust_decimal::Decimal;
use storage::identity::{MemoryIdentity, SourceInfo};

/// Whether a stored row carries the backfill flag.
fn is_backfilled(flags: QualityFlags) -> bool {
    flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)
}

/// Two rows of one bar close are distinguishable only if they differ in **both**
/// the backfill flag and the knowledge time; either signal alone does not survive
/// somebody dropping the other.
fn distinguishable(a: (QualityFlags, i64), b: (QualityFlags, i64)) -> bool {
    is_backfilled(a.0) != is_backfilled(b.0) && a.1 != b.1
}

/// The three sources this file needs, with the lags migration 0046 declares.
/// `alpaca_rest`'s 900 s is the number P-1.4 exists to keep distinct from the
/// crypto venues' 60 s.
fn identity() -> Arc<MemoryIdentity> {
    static ID: OnceLock<Arc<MemoryIdentity>> = OnceLock::new();
    ID.get_or_init(|| {
        let id = Arc::new(MemoryIdentity::new());
        id.add_source(
            "coinbase_rest",
            SourceInfo { source_id: 2, declared_vendor_lag: std::time::Duration::from_secs(60), live: false },
        );
        id.add_source(
            "alpaca_rest",
            SourceInfo { source_id: 5, declared_vendor_lag: std::time::Duration::from_secs(900), live: false },
        );
        id.add_source(
            "kraken_ws",
            SourceInfo { source_id: 9, declared_vendor_lag: std::time::Duration::ZERO, live: true },
        );
        storage::identity::install(id.clone());
        id
    })
    .clone()
}

async fn scratch(name: &str) -> Option<String> {
    // A gated E2E test that skips silently reports `ok` in 0.01 s and reads as
    // a pass. Everything in this file is a claim about a live database, so the
    // skip says so out loud — `--nocapture` shows it, and a run with no
    // ClickHouse cannot be mistaken for a green one.
    let Ok(base) = std::env::var("BACKTEST_E2E_CLICKHOUSE_URL") else {
        eprintln!("SKIPPED {name}: BACKTEST_E2E_CLICKHOUSE_URL is unset — this proves nothing");
        return None;
    };
    let admin = storage::clickhouse::connect(&base);
    admin.query(&format!("DROP DATABASE IF EXISTS {name}")).execute().await.unwrap();
    admin.query(&format!("CREATE DATABASE {name}")).execute().await.unwrap();
    let (scheme, rest) = base.split_once("://").unwrap();
    let host = rest.split('/').next().unwrap();
    let url = format!("{scheme}://{host}/{name}");
    storage::clickhouse::migrate::run_migrations(&url).await.unwrap();
    Some(url)
}

/// A bar close well in the past: its receipt today is meaningless, so the write
/// path takes the backfill branch and stamps the declared sentinel.
fn historical_close() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2023, 6, 1, 12, 1, 0).unwrap()
}

fn bar(close: DateTime<Utc>, price: i64) -> CollectedBar {
    let p = price.to_string();
    CollectedBar {
        available_time: close,
        sequence: close.timestamp() as u64,
        open: p.clone(),
        high: p.clone(),
        low: p.clone(),
        close: p,
        volume: "1".into(),
        trade_count: 1,
    }
}

/// P-1.5: a revised bar is a second row, and `as_of` separates the versions.
///
/// The `as_of` half is the part that was previously unreachable for backfilled
/// data: both versions shared the `close + vendor_lag` sentinel, so the
/// revision history existed in storage while no point-in-time read could return
/// the pre-revision value. A revision's knowledge time is now its receipt.
#[tokio::test(flavor = "multi_thread")]
async fn a_revision_is_a_second_row_and_as_of_returns_the_value_current_then() {
    let Some(url) = scratch("bf_revisions").await else { return };
    let id = identity();
    let store = BarStore::connect(&url);
    let close = historical_close();

    let first = store.write_collected("RV-USD", "coinbase", "coinbase_rest", Timeframe::Minutes1, &[bar(close, 100)]).await.unwrap();
    assert_eq!(first.inserted, 1);
    let between = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;

    let second = store.write_collected("RV-USD", "coinbase", "coinbase_rest", Timeframe::Minutes1, &[bar(close, 101)]).await.unwrap();
    assert_eq!((second.inserted, second.restated), (0, 1), "a changed cell is a revision, not a new bar");
    id.sync_dims(&url).await.unwrap();

    let range = (close - Duration::minutes(5), close + Duration::minutes(5));

    // Both versions are retained: two physical rows for one bar, even after a
    // full merge. No OPTIMIZE and no collapsing engine is involved.
    let ch = storage::clickhouse::connect(&url);
    let rows: u64 = ch
        .query("SELECT toUInt64(count()) FROM market_bar WHERE period_secs = 60")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(rows, 2, "a revision must not overwrite the row it restates (DA-16)");

    // As of before the revision was received, the original value stands.
    let earlier = BarStore::connect(&url)
        .as_of(between)
        .load_bars("RV-USD", Timeframe::Minutes1, range.0, range.1)
        .await
        .unwrap();
    assert_eq!(earlier.len(), 1);
    assert_eq!(earlier[0].close, Decimal::from(100), "an as_of before the revision must not see it");

    // And afterwards, the revision.
    let latest = BarStore::connect(&url).load_bars("RV-USD", Timeframe::Minutes1, range.0, range.1).await.unwrap();
    assert_eq!(latest.len(), 1, "one bar, resolved to its current revision");
    assert_eq!(latest[0].close, Decimal::from(101));
    assert!(latest[0].quality_flags.contains(QualityFlags::VENDOR_REVISED));
    assert!(
        latest[0].quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME),
        "a revision of a backfilled bar is still a backfilled bar"
    );
}

/// P-1.6: an observed row and a backfilled row of the same bar close are
/// distinguishable forever, by flag **and** by knowledge time.
///
/// This is the test that fails if the flag is dropped.
#[tokio::test(flavor = "multi_thread")]
async fn an_observed_row_and_a_backfilled_row_of_one_bar_stay_distinguishable() {
    let Some(url) = scratch("bf_distinguish").await else { return };
    let id = identity();
    let store = BarStore::connect(&url);

    // A bar that closed seconds ago, aligned to the minute: receipt now is a
    // real observation.
    let now = Utc::now();
    let close = now - Duration::seconds(5);
    let close = close - Duration::nanoseconds(close.timestamp_nanos_opt().unwrap() % 60_000_000_000);
    store.write_collected("DS-USD", "kraken", "kraken_ws", Timeframe::Minutes1, &[bar(close, 200)]).await.unwrap();

    // The same bar close, from a source polled long after: a backfill.
    let old = historical_close();
    store.write_collected("DS-USD", "kraken", "coinbase_rest", Timeframe::Minutes1, &[bar(old, 200)]).await.unwrap();
    id.sync_dims(&url).await.unwrap();

    #[derive(clickhouse::Row, serde::Deserialize)]
    struct R {
        event_ns: i64,
        knowledge_ns: i64,
        flags: u32,
    }
    let rows: Vec<R> = storage::clickhouse::connect(&url)
        .query(
            "SELECT toInt64(toUnixTimestamp64Nano(event_time)) AS event_ns,
                    toInt64(toUnixTimestamp64Nano(knowledge_time)) AS knowledge_ns,
                    quality_flags AS flags
               FROM market_bar WHERE period_secs = 60 ORDER BY event_time",
        )
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);

    let backfilled = rows.iter().find(|r| is_backfilled(QualityFlags(r.flags))).expect("the historical row is flagged");
    let observed = rows.iter().find(|r| !is_backfilled(QualityFlags(r.flags))).expect("the live row is not flagged");

    // The backfilled row carries exactly the declared sentinel: close + 60 s.
    assert_eq!(
        backfilled.knowledge_ns - backfilled.event_ns,
        120_000_000_000,
        "stored event_time is the bar OPEN, so close + 60 s is open + 120 s"
    );
    // The observed row's knowledge time is its receipt, not a sentinel.
    assert!(
        observed.knowledge_ns - observed.event_ns >= 60_000_000_000,
        "an observed bar cannot be known before it closes"
    );
    assert!(
        distinguishable(
            (QualityFlags(observed.flags), observed.knowledge_ns),
            (QualityFlags(backfilled.flags), backfilled.knowledge_ns)
        ),
        "the two rows must differ in BOTH the flag and the knowledge time"
    );
}

/// P-1.4's acceptance, against the store: two backfilled rows for the same bar
/// close from **different sources** carry **different** knowledge times,
/// because the lag is read per source rather than assumed equal.
#[tokio::test(flavor = "multi_thread")]
async fn two_sources_backfilling_one_bar_close_disagree_by_their_declared_lags() {
    let Some(url) = scratch("bf_two_lags").await else { return };
    let id = identity();
    let store = BarStore::connect(&url);
    let close = historical_close();

    // Same symbol, same bar close, two venues so the rows do not collide.
    store.write_collected("TL-USD", "coinbase", "coinbase_rest", Timeframe::Minutes1, &[bar(close, 300)]).await.unwrap();
    store.write_collected("TL-USD", "alpaca", "alpaca_rest", Timeframe::Minutes1, &[bar(close, 300)]).await.unwrap();
    id.sync_dims(&url).await.unwrap();

    #[derive(clickhouse::Row, serde::Deserialize)]
    struct R {
        source_id: i32,
        lag_ns: i64,
        flags: u32,
    }
    let rows: Vec<R> = storage::clickhouse::connect(&url)
        .query(
            "SELECT source_id,
                    toInt64(toUnixTimestamp64Nano(knowledge_time) - toUnixTimestamp64Nano(event_time)) AS lag_ns,
                    quality_flags AS flags
               FROM market_bar WHERE period_secs = 60 ORDER BY source_id",
        )
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "two venues, two rows");
    assert!(rows.iter().all(|r| is_backfilled(QualityFlags(r.flags))), "both are backfills");

    let coinbase = rows.iter().find(|r| r.source_id == 2).expect("coinbase_rest");
    let alpaca = rows.iter().find(|r| r.source_id == 5).expect("alpaca_rest");
    // open + period + declared lag.
    assert_eq!(coinbase.lag_ns, 60_000_000_000 + 60_000_000_000);
    assert_eq!(alpaca.lag_ns, 60_000_000_000 + 900_000_000_000);
    assert_ne!(
        coinbase.lag_ns, alpaca.lag_ns,
        "assuming the lags equal is a fourteen-minute lookahead on every Alpaca bar"
    );

    // And a dataset mixing the two sources is embargoed by the larger lag.
    let embargo = features::walk_forward::embargo_inputs_with_sources(&[], 1, &[60_000, 900_000]);
    assert_eq!(embargo.max_knowledge_lag_ms, 900_000);
}

/// The sink the backfill engine actually writes through lands rows in
/// `market_bar` with the flag and the sentinel, and a re-run of the same page
/// writes nothing new.
#[tokio::test(flavor = "multi_thread")]
async fn the_backfill_sink_writes_flagged_rows_and_a_repeat_page_is_free() {
    let Some(url) = scratch("bf_sink").await else { return };
    let id = identity();
    let sink = BarStoreSink::new(Arc::new(BarStore::connect(&url)));
    let close = historical_close();
    let bars: Vec<BackfillBar> = (0..10)
        .map(|i| BackfillBar {
            close_time: close + Duration::minutes(i),
            open: "100".into(),
            high: "100".into(),
            low: "100".into(),
            close: "100".into(),
            volume: "1".into(),
            trade_count: 1,
        })
        .collect();

    let first = sink.write_page("coinbase_rest", "coinbase", "SK-USD", "1m", &bars).await.unwrap();
    assert_eq!((first.inserted, first.rejected), (10, 0));

    // Resuming over an already-covered range must be free, or every restart
    // would rewrite history it already holds.
    let again = sink.write_page("coinbase_rest", "coinbase", "SK-USD", "1m", &bars).await.unwrap();
    assert_eq!((again.inserted, again.unchanged, again.restated), (0, 10, 0));
    id.sync_dims(&url).await.unwrap();

    let loaded = BarStore::connect(&url)
        .load_bars("SK-USD", Timeframe::Minutes1, close - Duration::minutes(1), close + Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(loaded.len(), 10);
    assert!(
        loaded.iter().all(|b| b.quality_flags.contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME)),
        "every row the backfill writes says so"
    );
}
