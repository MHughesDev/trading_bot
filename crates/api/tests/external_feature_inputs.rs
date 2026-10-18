//! Set P P-3.6 against real stores: a feature over a macro series or an EDGAR tag is
//! computed from the **point-in-time rows the API hands in**, and the column at each
//! bar is what was knowable at that bar.
//!
//! `api::features_compute::load_external_rows` is what `GET /api/data/features` calls.
//! This drives it over a planted revision on both stores and asserts the column steps
//! to the revised value only at the bar after the revision became knowable.
//!
//! Needs a Postgres carrying migrations 0044/0046/0104/0156 and a ClickHouse:
//!
//! ```text
//! EXTERNAL_INPUTS_PG_URL=postgres://postgres:postgres@127.0.0.1:5433/tp_ext \
//! EXTERNAL_INPUTS_CLICKHOUSE_URL=http://trading:trading@localhost:8123 \
//!   cargo test -j 2 -p api --test external_feature_inputs -- --ignored --test-threads=1
//! ```

use std::sync::Arc;

use api::features_compute::load_external_rows;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use dataplane::feature::FeatureRow;
use dataplane::vintage::MacroObservation;
use features::external_features::{backfill_column_external, resolve_external, ExternalFeature};
use rust_decimal::Decimal;
use storage::clickhouse::{alt, connect, migrate};

const HOUR_NS: i64 = 3_600 * 1_000_000_000;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn with_database(url: &str, database: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let (creds, host_path) = match rest.rfind('@') {
        Some(at) => (format!("{}@", &rest[..at]), &rest[at + 1..]),
        None => (String::new(), rest),
    };
    let host = host_path.split('/').next().unwrap_or(host_path);
    format!("{scheme}://{creds}{host}/{database}")
}

/// Hourly bars from `start`, one per hour for `n` hours.
fn bars(start: DateTime<Utc>, n: usize) -> Vec<FeatureRow> {
    (0..n)
        .map(|i| FeatureRow {
            ts_ns: start.timestamp_nanos_opt().unwrap() + i as i64 * HOUR_NS,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        })
        .collect()
}

fn feat(name: &str) -> Arc<dyn ExternalFeature> {
    resolve_external(name, 3600)
        .unwrap()
        .expect("an external name")
}

/// The value of `column` at the bar whose timestamp is `at`.
fn at(rows: &[FeatureRow], column: &[Option<f64>], at: DateTime<Utc>) -> Option<f64> {
    let ns = at.timestamp_nanos_opt().unwrap();
    let i = rows
        .iter()
        .position(|r| r.ts_ns == ns)
        .expect("a bar at that instant");
    column[i]
}

#[tokio::test]
#[ignore = "needs EXTERNAL_INPUTS_PG_URL (0044/0046/0104/0156) and EXTERNAL_INPUTS_CLICKHOUSE_URL"]
async fn a_column_over_a_revised_macro_series_and_a_restated_fact_steps_only_when_the_revision_is_knowable(
) {
    let pg = sqlx::PgPool::connect(
        &std::env::var("EXTERNAL_INPUTS_PG_URL").expect("EXTERNAL_INPUTS_PG_URL"),
    )
    .await
    .unwrap();
    let ch_base =
        std::env::var("EXTERNAL_INPUTS_CLICKHOUSE_URL").expect("EXTERNAL_INPUTS_CLICKHOUSE_URL");
    let db = format!("_ext_inputs_{}", std::process::id());
    connect(&ch_base)
        .query(&format!("CREATE DATABASE IF NOT EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    let ch = with_database(&ch_base, &db);
    migrate::run_migrations(&ch).await.expect("schema");

    // DGS10 for 9 June: 4.0 published 10 June, revised to 4.2 on 12 June. The 13 June
    // observation is published on 14 June as "." (missing).
    alt::write_macro(
        &ch,
        21,
        &[
            MacroObservation::derived(
                "DGS10",
                d(2025, 6, 9),
                d(2025, 6, 10),
                Some(Decimal::new(40, 1)),
            ),
            MacroObservation::derived(
                "DGS10",
                d(2025, 6, 9),
                d(2025, 6, 12),
                Some(Decimal::new(42, 1)),
            ),
            MacroObservation::derived("DGS10", d(2025, 6, 13), d(2025, 6, 14), None),
        ],
    )
    .await
    .unwrap();

    // A fact of our own CIK per run (the table is append-only): 100 accepted at
    // 20:05 on 11 June, restated to 110 at 21:00 on 13 June.
    let cik = 800_000_000 + (std::process::id() % 90_000_000);
    let original = Utc.with_ymd_and_hms(2025, 6, 11, 20, 5, 0).unwrap();
    let restated = Utc.with_ymd_and_hms(2025, 6, 13, 21, 0, 0).unwrap();
    for (acc, accepted, v) in [
        ("0000000000-25-000001", original, 100),
        ("0000000000-25-000002", restated, 110),
    ] {
        sqlx::query(
            "INSERT INTO dataplane.fundamentals_fact
               (cik, tag, unit, period_start, period_end, fiscal_year, fiscal_period, form,
                accession, value, accepted_at, knowledge, source_id)
             SELECT $1,'Revenues','USD','2025-01-01','2025-03-31',2025,'Q1','10-Q',$2,$3,$4,'observed', s.source_id
               FROM dataplane.source s WHERE s.name = 'sec_edgar'",
        )
        .bind(i64::from(cik))
        .bind(acc)
        .bind(Decimal::from(v))
        .bind(accepted)
        .execute(&pg)
        .await
        .unwrap();
    }

    let start = Utc.with_ymd_and_hms(2025, 6, 9, 0, 0, 0).unwrap();
    let rows = bars(start, 24 * 7);
    let macro_f = feat("macro_dgs10");
    let fact_f = feat(&format!("fundamental_{cik}_Revenues"));
    let loaded = load_external_rows(
        &pg,
        &ch,
        &[Arc::clone(&macro_f), Arc::clone(&fact_f)],
        start,
        start + chrono::Duration::days(7),
        Utc.with_ymd_and_hms(2025, 7, 1, 0, 0, 0).unwrap(),
    )
    .await
    .unwrap();
    // Every vintage and every filing comes through: the choice is per decision.
    assert_eq!(
        loaded.macro_rows.len(),
        3,
        "all three vintages, not one per observation"
    );
    assert_eq!(loaded.facts.len(), 2, "both filings, not only the latest");

    let m = backfill_column_external(macro_f.as_ref(), &rows, &loaded.inputs());
    let f = backfill_column_external(fact_f.as_ref(), &rows, &loaded.inputs());
    let t = |day, h| Utc.with_ymd_and_hms(2025, 6, day, h, 0, 0).unwrap();

    // Macro: a vintage is knowable at the end of its publication day.
    assert_eq!(
        at(&rows, &m, t(10, 12)),
        None,
        "published today, not yet knowable"
    );
    assert_eq!(
        at(&rows, &m, t(11, 12)),
        Some(4.0),
        "the ORIGINAL before the revision"
    );
    assert_eq!(
        at(&rows, &m, t(12, 12)),
        Some(4.0),
        "revised today, not yet knowable"
    );
    assert_eq!(
        at(&rows, &m, t(13, 12)),
        Some(4.2),
        "the revision once its day has ended"
    );
    // The newest period reported missing: absent, never the older period's 4.2.
    assert_eq!(at(&rows, &m, t(15, 12)), None);

    // Fundamentals: absent before acceptance, the original at acceptance, the
    // restated figure only after its own acceptance.
    assert_eq!(at(&rows, &f, t(11, 19)), None, "filed after this bar");
    assert_eq!(
        at(&rows, &f, t(11, 21)),
        Some(100.0),
        "the original once accepted"
    );
    assert_eq!(
        at(&rows, &f, t(13, 20)),
        Some(100.0),
        "the restatement is not yet knowable"
    );
    assert_eq!(
        at(&rows, &f, t(13, 22)),
        Some(110.0),
        "the restated figure after its acceptance"
    );

    // The same table is readable as the platform's own role (a missing GRANT passes
    // as the owner and fails live).
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM dataplane.fundamentals_fact WHERE cik = $1")
            .bind(i64::from(cik))
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(n, 2);
    tx.rollback().await.unwrap();

    connect(&ch_base)
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
}
