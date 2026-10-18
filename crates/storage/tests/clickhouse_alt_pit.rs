//! Set P Phase 3 against a real ClickHouse: each alternative-data store returns
//! what was knowable at `as_of`, proven over a planted revision (P-3.1, P-3.4,
//! P-3.5; DATA-005 §10).
//!
//! Gated on `STORAGE_ALT_CLICKHOUSE_URL`, like the other live-ClickHouse suites in
//! this crate; **a skipped run is not evidence**, and the phase file records the
//! run that was made. Writes only to a throwaway database, which it drops.
//!
//! ```bash
//! STORAGE_ALT_CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//!   cargo test -j 2 -p storage --test clickhouse_alt_pit -- --nocapture --test-threads=1
//! ```

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use dataplane::source::Knowledge;
use dataplane::text::TextItem;
use dataplane::vintage::MacroObservation;
use rust_decimal::Decimal;
use storage::clickhouse::{alt, connect, migrate};

fn with_database(url: &str, database: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let (creds, host_path) = match rest.rfind('@') {
        Some(at) => (format!("{}@", &rest[..at]), &rest[at + 1..]),
        None => (String::new(), rest),
    };
    let host = host_path.split('/').next().unwrap_or(host_path);
    format!("{scheme}://{creds}{host}/{database}")
}

async fn scratch(tag: &str) -> Option<(String, String)> {
    let base = std::env::var("STORAGE_ALT_CLICKHOUSE_URL").ok()?;
    let db = format!("_alt_pit_{tag}_{}", std::process::id());
    connect(&base)
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    let url = with_database(&base, &db);
    migrate::run_migrations(&url).await.expect("schema");
    Some((base, url))
}

async fn drop_db(base: &str, url: &str) {
    let db = url.rsplit('/').next().unwrap();
    connect(base)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// P-3.1 clauses 1 and 2: a known revision returns the original before it and the
/// revision after; the same query twice gives the same answer.
#[tokio::test]
async fn a_macro_revision_is_invisible_before_its_vintage() {
    let Some((base, url)) = scratch("macro").await else {
        eprintln!("STORAGE_ALT_CLICKHOUSE_URL unset - skipping (not evidence)");
        return;
    };
    // GDP for 2025-01-01: first published 2025-04-30 at 100, revised 2025-05-29 to
    // 103; and a later quarter whose newest vintage reported missing.
    let rows = vec![
        MacroObservation::derived("GDPC1", d(2025, 1, 1), d(2025, 4, 30), Some(Decimal::from(100))),
        MacroObservation::derived("GDPC1", d(2025, 1, 1), d(2025, 5, 29), Some(Decimal::from(103))),
        MacroObservation::derived("GDPC1", d(2025, 4, 1), d(2025, 7, 30), Some(Decimal::from(105))),
        MacroObservation::derived("GDPC1", d(2025, 4, 1), d(2025, 8, 28), None),
    ];
    assert_eq!(alt::write_macro(&url, 21, &rows).await.unwrap(), 4);

    let read = |as_of| alt::macro_as_of(&url, "GDPC1", d(2024, 1, 1), d(2026, 1, 1), as_of);
    let first_day = read(Utc.with_ymd_and_hms(2025, 4, 30, 12, 0, 0).unwrap()).await.unwrap();
    assert!(
        first_day.is_empty(),
        "a vintage is knowable at the END of its publication day: {first_day:?}"
    );

    let mid_may = Utc.with_ymd_and_hms(2025, 5, 15, 0, 0, 0).unwrap();
    let a = read(mid_may).await.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].value, Some(Decimal::from(100)), "the ORIGINAL before the revision");
    assert_ne!(a[0].quality_flags & 0x8000, 0, "a derived vintage carries the backfill flag");
    assert_eq!(read(mid_may).await.unwrap(), a, "the same query twice, the same answer");

    let sept = Utc.with_ymd_and_hms(2025, 9, 1, 0, 0, 0).unwrap();
    let b = read(sept).await.unwrap();
    assert_eq!(b[0].value, Some(Decimal::from(103)), "the revision after it");
    assert_eq!(b[1].observation_date, d(2025, 4, 1));
    assert_eq!(
        b[1].value, None,
        "a vintage that reported missing must not fall back to the previous value"
    );
    drop_db(&base, &url).await;
}

/// P-3.6: the reader that feeds a feature column returns the WHOLE revision history
/// up to `as_of` (not one vintage per observation), keeps a missing vintage as
/// `None`, and never returns a vintage that was not yet knowable at `as_of`.
#[tokio::test]
async fn the_feature_reader_returns_every_vintage_knowable_by_as_of() {
    let Some((base, url)) = scratch("vint").await else {
        eprintln!("STORAGE_ALT_CLICKHOUSE_URL unset - skipping (not evidence)");
        return;
    };
    let rows = vec![
        MacroObservation::derived("GDPC1", d(2025, 1, 1), d(2025, 4, 30), Some(Decimal::from(100))),
        MacroObservation::derived("GDPC1", d(2025, 1, 1), d(2025, 5, 29), Some(Decimal::from(103))),
        MacroObservation::derived("GDPC1", d(2025, 4, 1), d(2025, 7, 30), Some(Decimal::from(105))),
        MacroObservation::derived("GDPC1", d(2025, 4, 1), d(2025, 8, 28), None),
    ];
    alt::write_macro(&url, 21, &rows).await.unwrap();
    let read = |as_of| alt::macro_vintages(&url, "GDPC1", d(2024, 1, 1), d(2026, 1, 1), as_of);

    let sept = Utc.with_ymd_and_hms(2025, 9, 1, 0, 0, 0).unwrap();
    let all = read(sept).await.unwrap();
    assert_eq!(all.len(), 4, "every vintage, including the superseded ones");
    assert_eq!(all[0].value, Some(Decimal::from(100)), "the original is still there");
    assert_eq!(all[1].value, Some(Decimal::from(103)));
    assert_eq!(all[3].value, None, "a vintage that reported missing stays missing");
    assert!(!all[0].knowledge.is_forward_evidence(), "a derived stamp is read back as derived");

    // Mid-May: only the first vintage is knowable; the revision must not leak.
    let mid_may = Utc.with_ymd_and_hms(2025, 5, 15, 0, 0, 0).unwrap();
    let early = read(mid_may).await.unwrap();
    assert_eq!(early.len(), 1);
    assert_eq!(early[0].value, Some(Decimal::from(100)));

    // And the per-instant choice over what came back is the one the runtime makes.
    let seen = dataplane::vintage::series_as_of(&all, "GDPC1", sept);
    assert_eq!(seen.last().unwrap().value, None);
    drop_db(&base, &url).await;
}

/// P-3.4: Kalshi probabilities are readable with the cutoff applied; a backfilled
/// candlestick is not readable at its own close.
#[tokio::test]
async fn a_prediction_market_row_is_not_readable_before_its_knowledge_time() {
    let Some((base, url)) = scratch("pm").await else {
        eprintln!("STORAGE_ALT_CLICKHOUSE_URL unset - skipping (not evidence)");
        return;
    };
    let close = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
    let derived = Knowledge::Derived {
        at: close + Duration::seconds(60),
        from_event_time: close,
        declared_lag: std::time::Duration::from_secs(60),
    };
    let row = alt::ProbabilityRow::new(
        "kalshi",
        "FED-25DEC-T4.00",
        close,
        derived,
        Decimal::new(42, 2),
        10,
        100,
        13,
    );
    alt::write_probabilities(&url, &[row]).await.unwrap();
    let (from, to) = (close - Duration::hours(1), close + Duration::hours(1));
    let at_close = alt::probabilities_as_of(&url, "kalshi", "FED-25DEC-T4.00", from, to, close)
        .await
        .unwrap();
    assert!(at_close.is_empty(), "not readable at its own close");
    let later = alt::probabilities_as_of(
        &url,
        "kalshi",
        "FED-25DEC-T4.00",
        from,
        to,
        close + Duration::seconds(60),
    )
    .await
    .unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].close_probability, Decimal::new(42, 2));
    assert_ne!(later[0].quality_flags & 0x8000, 0);
    let bad = alt::ProbabilityRow::new("kalshi", "X", close, derived, Decimal::from(2), 0, 0, 13);
    assert!(
        alt::write_probabilities(&url, &[bad]).await.is_err(),
        "a probability outside [0, 1] is refused rather than stored"
    );
    drop_db(&base, &url).await;
}

/// P-3.5 clauses 1 and 3: a corrected article is two rows, an as-of read returns
/// the version current then, and nothing is returned before it was available.
#[tokio::test]
async fn a_corrected_article_is_two_rows_and_reads_as_it_stood() {
    let Some((base, url)) = scratch("text").await else {
        eprintln!("STORAGE_ALT_CLICKHOUSE_URL unset - skipping (not evidence)");
        return;
    };
    let t0 = Utc.with_ymd_and_hms(2026, 9, 1, 14, 0, 0).unwrap();
    let mut first = TextItem::first("alpaca:1", 22, "1", "Company beats", t0, t0, 0);
    first.symbols = vec!["AAPL".into()];
    let corrected = first.revise(
        "Company misses (corrected)",
        None,
        t0 + Duration::hours(2),
        t0 + Duration::hours(2),
        0,
    );
    alt::write_text_items(&url, &[first.clone(), corrected])
        .await
        .unwrap();

    let (from, to) = (t0 - Duration::days(1), t0 + Duration::days(1));
    let before = alt::text_as_of(&url, Some("AAPL"), from, to, t0 - Duration::seconds(1), 50)
        .await
        .unwrap();
    assert!(before.is_empty(), "nothing before publication");
    let between = alt::text_as_of(&url, Some("AAPL"), from, to, t0 + Duration::hours(1), 50)
        .await
        .unwrap();
    assert_eq!(between.len(), 1);
    assert_eq!(between[0].revision, 0);
    assert_eq!(between[0].headline, "Company beats", "the version current at the time");
    let after = alt::text_as_of(&url, Some("AAPL"), from, to, t0 + Duration::hours(3), 50)
        .await
        .unwrap();
    assert_eq!(after[0].revision, 1);
    assert_eq!(after[0].headline, "Company misses (corrected)");

    #[derive(clickhouse::Row, serde::Deserialize)]
    struct N {
        n: u64,
    }
    let n: N = connect(&url)
        .query("SELECT count() AS n FROM text_items FINAL WHERE item_id = 'alpaca:1'")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n.n, 2, "a correction is a new row, not an update");
    drop_db(&base, &url).await;
}
