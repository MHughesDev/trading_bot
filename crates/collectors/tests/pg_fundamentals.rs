//! DATA-005 §10.4 against PostgreSQL (Set P P-3.2): **a fundamentals request as of
//! date t never returns a value filed after t**, over the real table and the real
//! read path, with a fact whose period precedes its acceptance by a quarter.
//!
//! Needs a database carrying migrations 0046, 0104 and 0156:
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/p3_scratch \
//!   cargo test -j 2 -p collectors --test pg_fundamentals -- --ignored
//! ```

use chrono::{NaiveDate, TimeZone, Utc};
use collectors::fundamentals::store;
use dataplane::fundamentals::FundamentalFact;
use dataplane::source::Knowledge;
use rust_decimal::Decimal;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

fn fact(cik: u32, accession: &str, accepted: chrono::DateTime<Utc>, value: i64) -> FundamentalFact {
    FundamentalFact {
        cik,
        tag: "Revenues".into(),
        unit: "USD".into(),
        period_start: Some(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
        period_end: NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
        fiscal_year: Some(2025),
        fiscal_period: Some("Q1".into()),
        form: "10-Q".into(),
        accession: accession.into(),
        value: Decimal::from(value),
        knowledge: Knowledge::Observed { at: accepted },
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database with 0046/0104/0156"]
async fn a_request_as_of_t_never_returns_a_value_filed_after_t() {
    let pg = pool().await;
    // A CIK of our own per run, so reruns do not collide with the append-only table.
    let cik = 900_000_000 + (std::process::id() % 90_000_000);
    let original = Utc.with_ymd_and_hms(2025, 5, 2, 20, 5, 0).unwrap(); // a quarter after the period
    let restated = Utc.with_ymd_and_hms(2025, 8, 1, 21, 0, 0).unwrap();
    let n = store::insert_facts(
        &pg,
        &[
            fact(cik, "0000000000-25-000001", original, 100),
            fact(cik, "0000000000-25-000002", restated, 110),
        ],
    )
    .await
    .unwrap();
    assert_eq!(n, 2);

    // After the period ended, before it was filed: nothing. Reading by period
    // would have returned 100 here — the lookahead this table exists to refuse.
    let mid_april = Utc.with_ymd_and_hms(2025, 4, 15, 0, 0, 0).unwrap();
    assert!(
        store::facts_as_of(&pg, i64::from(cik), "Revenues", mid_april)
            .await
            .unwrap()
            .is_empty()
    );
    // One second before acceptance: still nothing.
    let just_before = original - chrono::Duration::seconds(1);
    assert!(
        store::facts_as_of(&pg, i64::from(cik), "Revenues", just_before)
            .await
            .unwrap()
            .is_empty()
    );
    // Once accepted: the original.
    let after = store::facts_as_of(&pg, i64::from(cik), "Revenues", original)
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].value, Decimal::from(100));
    // After the restatement: the restated value, and the original is still stored.
    let later = store::facts_as_of(&pg, i64::from(cik), "Revenues", restated)
        .await
        .unwrap();
    assert_eq!(later[0].value, Decimal::from(110));
    let (stored,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM dataplane.fundamentals_fact WHERE cik = $1")
            .bind(i64::from(cik))
            .fetch_one(&pg)
            .await
            .unwrap();
    assert_eq!(stored, 2, "a restatement is a new row, never an overwrite");

    // Idempotent: the same facts again write nothing.
    assert_eq!(
        store::insert_facts(&pg, &[fact(cik, "0000000000-25-000001", original, 100)])
            .await
            .unwrap(),
        0
    );

    // Append-only, for every role.
    assert!(
        sqlx::query("UPDATE dataplane.fundamentals_fact SET value = 1 WHERE cik = $1")
            .bind(i64::from(cik))
            .execute(&pg)
            .await
            .is_err()
    );

    // The platform's role can write and read it (tests run as owner, so a missing
    // GRANT would otherwise pass here and fail live).
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT count(*) FROM dataplane.fundamentals_fact WHERE cik = $1")
        .bind(i64::from(cik))
        .execute(&mut *tx)
        .await
        .expect("app_role can read fundamentals_fact");
    sqlx::query(
        "INSERT INTO dataplane.fundamentals_fact (cik, tag, unit, period_end, form, accession, value, accepted_at, knowledge, source_id)
         VALUES ($1, 'Assets', 'USD', '2025-03-31', '10-Q', 'x', 1, '2025-05-02', 'derived', 20)",
    )
    .bind(i64::from(cik))
    .execute(&mut *tx)
    .await
    .expect("app_role can insert fundamentals_fact");
    tx.rollback().await.unwrap();

    // A filing accepted before its period ended is refused by the table itself.
    assert!(sqlx::query(
        "INSERT INTO dataplane.fundamentals_fact (cik, tag, unit, period_end, form, accession, value, accepted_at, knowledge, source_id)
         VALUES ($1, 'Assets', 'USD', '2025-03-31', '10-Q', 'early', 1, '2025-03-01', 'derived', 20)",
    )
    .bind(i64::from(cik))
    .execute(&pg)
    .await
    .is_err());
}
