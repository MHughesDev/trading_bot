//! P-0.3 clause 3 against a real ClickHouse: **a restart resumes without
//! duplicating a row** — writing the same settlement twice stores it once, while a
//! changed rate for that settlement is kept as a restatement.
//!
//! Found on the first live run (2026-10-03): `knowledge_time` is in
//! `funding_rates`' sorting key, so every re-poll of an unchanged settlement was a
//! new row and every realized rate was stored twice.
//!
//! Gated on `STORAGE_ALT_CLICKHOUSE_URL`; **a skipped run is not evidence.**

use chrono::{Duration, TimeZone, Utc};
use dataplane::source::Knowledge;
use rust_decimal::Decimal;
use storage::clickhouse::forward::{write_funding, FundingKind, FundingRow};
use storage::clickhouse::{connect, migrate};

fn with_database(url: &str, database: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let (creds, host_path) = match rest.rfind('@') {
        Some(at) => (format!("{}@", &rest[..at]), &rest[at + 1..]),
        None => (String::new(), rest),
    };
    let host = host_path.split('/').next().unwrap_or(host_path);
    format!("{scheme}://{creds}{host}/{database}")
}

#[derive(clickhouse::Row, serde::Deserialize)]
struct N {
    n: u64,
}

#[tokio::test]
async fn re_polling_an_unchanged_settlement_writes_nothing_and_a_restatement_writes_once() {
    let Ok(base) = std::env::var("STORAGE_ALT_CLICKHOUSE_URL") else {
        eprintln!("STORAGE_ALT_CLICKHOUSE_URL unset - skipping (not evidence)");
        return;
    };
    let db = format!("_forward_idem_{}", std::process::id());
    connect(&base)
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    let url = with_database(&base, &db);
    migrate::run_migrations(&url).await.unwrap();

    let settle = Utc.with_ymd_and_hms(2026, 10, 3, 17, 0, 0).unwrap();
    let row = |polled_at: chrono::DateTime<Utc>, rate: i64| FundingRow {
        venue: "kraken_futures".into(),
        instrument: "PF_XBTUSD".into(),
        kind: FundingKind::Realized,
        event_time: settle,
        venue_ts: None,
        ingest_time: polled_at,
        knowledge: Knowledge::Observed { at: polled_at },
        rate: Decimal::new(rate, 6),
        relative_rate: None,
        source_id: 17,
    };
    let first = settle + Duration::minutes(37);
    assert_eq!(write_funding(&url, &[row(first, 52)]).await.unwrap(), 1);
    // The restart / the next poll: same settlement, same rate, later receipt.
    assert_eq!(
        write_funding(&url, &[row(first + Duration::minutes(15), 52)])
            .await
            .unwrap(),
        0,
        "an unchanged settlement must not be written twice"
    );
    // A restatement: same settlement, different rate. Kept, as a new row.
    assert_eq!(
        write_funding(&url, &[row(first + Duration::minutes(30), 53)])
            .await
            .unwrap(),
        1
    );
    let n: N = connect(&url)
        .query("SELECT count() AS n FROM funding_rates")
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n.n, 2, "the original and its restatement, nothing else");

    connect(&base)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
}
