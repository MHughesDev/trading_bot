//! P-2.2 end to end: a series in ClickHouse is graded by the `dataplane::qc`
//! battery and the report lands in `data_qc_reports`, with its reasons, and is read
//! back as the grade a manifest carries.
//!
//! Gated on `STORAGE_QC_CLICKHOUSE_URL` (a throwaway ClickHouse database is
//! created and dropped) and `STORAGE_QC_DATABASE_URL` (a scratch Postgres holding
//! migration 0100). **A skipped run is not evidence.**
//!
//! ```bash
//! STORAGE_QC_CLICKHOUSE_URL=http://trading:trading@localhost:8123/trading \
//! STORAGE_QC_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/p3_scratch \
//!   cargo test -j 2 -p storage --test qc_reports_live -- --nocapture
//! ```

use chrono::{Duration, TimeZone, Utc};
use dataplane::qc::QcConfig;
use storage::clickhouse::{connect, migrate, qc_reports};

fn with_database(url: &str, database: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let (creds, host_path) = match rest.rfind('@') {
        Some(at) => (format!("{}@", &rest[..at]), &rest[at + 1..]),
        None => (String::new(), rest),
    };
    let host = host_path.split('/').next().unwrap_or(host_path);
    format!("{scheme}://{creds}{host}/{database}")
}

#[tokio::test]
async fn a_graded_series_is_persisted_with_its_reasons_and_read_back() {
    let (Ok(base), Ok(pg_url)) = (
        std::env::var("STORAGE_QC_CLICKHOUSE_URL"),
        std::env::var("STORAGE_QC_DATABASE_URL"),
    ) else {
        eprintln!("STORAGE_QC_* unset - skipping (not evidence)");
        return;
    };
    let db = format!("_qc_reports_{}", std::process::id());
    // The report table is append-only, so each run grades a name of its own.
    let sym = format!("QC-TEST-{}", std::process::id());
    connect(&base)
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    let url = with_database(&base, &db);
    migrate::run_migrations(&url).await.unwrap();
    let c = connect(&url);
    c.query("INSERT INTO venue_dim VALUES (1, 'coinbase', 1)").execute().await.unwrap();
    c.query(&format!("INSERT INTO instrument_symbol_dim VALUES (1005, 1, '{sym}', '2000-01-01', NULL, '2000-01-01')"))
        .execute()
        .await
        .unwrap();

    // One day of 1m bars with a planted 200-minute hole: ~13.9 % missing -> D.
    let t0 = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let mut values = Vec::new();
    for i in 0..1440_i64 {
        if (600..800).contains(&i) {
            continue;
        }
        let open = t0 + Duration::minutes(i);
        let s = open.timestamp();
        values.push(format!(
            "(1005, 1, 60, fromUnixTimestamp64Nano(toInt64({s})*1000000000), NULL, now64(9), \
             fromUnixTimestamp64Nano(toInt64({k})*1000000000), 100, 101, 99, 100.5, 3, NULL, NULL, NULL, NULL, NULL, NULL, 32768, 0, 2)",
            k = s + 120
        ));
    }
    c.query(&format!("INSERT INTO market_bar VALUES {}", values.join(",")))
        .execute()
        .await
        .expect("seed bars");

    let pg = sqlx::PgPool::connect(&pg_url).await.unwrap();
    let cfg = QcConfig::default();
    let (id, report) = qc_reports::grade_and_persist(
        &pg,
        &url,
        &sym,
        "coinbase",
        "1m",
        60,
        t0,
        t0 + Duration::days(1),
        1440,
        &cfg,
    )
    .await
    .expect("graded and persisted");
    assert_eq!(report.bars_examined, 1240);
    assert_eq!(report.graded.grade.as_str(), "D", "{:?}", report.graded.reasons);

    // Idempotent under one config version: the same window returns the same id.
    let (again, _) = qc_reports::grade_and_persist(
        &pg, &url, &sym, "coinbase", "1m", 60, t0, t0 + Duration::days(1), 1440, &cfg,
    )
    .await
    .unwrap();
    assert_eq!(again, id);

    // Read back as the manifest reads it, reasons included.
    let g = qc_reports::latest_grade(&pg, &sym, "1m").await.unwrap().unwrap();
    assert_eq!(g.report_id, id);
    assert_eq!(g.grade, "D");
    assert!(g.reasons.as_array().is_some_and(|r| !r.is_empty()), "{:?}", g.reasons);
    assert_eq!(g.config_version, cfg.version());
    assert!(qc_reports::latest_grade(&pg, "NEVER-GRADED", "1m").await.unwrap().is_none());

    // The platform's role can write and read the table.
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    sqlx::query("SELECT count(*) FROM data_qc_reports")
        .execute(&mut *tx)
        .await
        .expect("app_role reads data_qc_reports");
    tx.rollback().await.unwrap();

    connect(&base)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
}
