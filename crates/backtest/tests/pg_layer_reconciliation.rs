//! The reconciliation record's constraints against PostgreSQL (Set M M-4.1).
//!
//! Self-contained: it applies `migrations/0205_strategy_reconciliation.sql` itself and, because
//! that table references `strategy_revision`, creates a one-column stand-in for it **only when the
//! database has none**. It is a scratch-database test:
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p backtest --test pg_layer_reconciliation -- --ignored --test-threads 1
//! ```
//!
//! Tests connect as the owner, which cannot see a missing GRANT, so the platform's role is
//! exercised with `SET LOCAL ROLE app_role` (the pattern of `pg_universe.rs`).

use sqlx::{Executor, PgPool, Row};

const MIGRATION: &str = include_str!("../../../migrations/0205_strategy_reconciliation.sql");

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

async fn pool() -> PgPool {
    let pg = PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects");
    sqlx::raw_sql(
        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_role') \
         THEN CREATE ROLE app_role NOLOGIN; END IF; END $$;",
    )
    .execute(&pg)
    .await
    .unwrap();
    let have: bool = sqlx::query("SELECT to_regclass('strategy_revision') IS NOT NULL")
        .fetch_one(&pg)
        .await
        .unwrap()
        .get(0);
    #[allow(clippy::if_not_else)] // the create-it branch reads first
    if !have {
        sqlx::raw_sql(
            "CREATE TABLE strategy_revision (revision_hash TEXT PRIMARY KEY \
             CHECK (revision_hash ~ '^[0-9a-f]{64}$'));",
        )
        .execute(&pg)
        .await
        .unwrap();
        for h in [A, B] {
            sqlx::query("INSERT INTO strategy_revision VALUES ($1)")
                .bind(h)
                .execute(&pg)
                .await
                .unwrap();
        }
    } else {
        let n: i64 =
            sqlx::query("SELECT count(*) FROM strategy_revision WHERE revision_hash = ANY($1)")
                .bind(vec![A.to_string(), B.to_string()])
                .fetch_one(&pg)
                .await
                .unwrap()
                .get(0);
        assert_eq!(
            n, 2,
            "this database has a real strategy_revision without the two test hashes; use a scratch database"
        );
    }
    sqlx::raw_sql(MIGRATION)
        .execute(&pg)
        .await
        .expect("the migration applies, and applies twice");
    pg
}

const INSERT: &str = "INSERT INTO strategy_reconciliation \
    (layer1_revision, layer2_revision, experiment_id, n_decisions, agreement, pnl_tracking_error, \
     thresholds, explained_diffs, unexplained_count, passed) \
    VALUES ($1, $2, $3, 100, $4, $5, $6, '[]'::jsonb, $7, $8)";

#[allow(clippy::too_many_arguments)]
async fn insert(
    pg: &PgPool,
    experiment: &str,
    agreement: f64,
    te: f64,
    thresholds: serde_json::Value,
    unexplained: i32,
    passed: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(INSERT)
        .bind(A)
        .bind(B)
        .bind(experiment)
        .bind(agreement)
        .bind(te)
        .bind(thresholds)
        .bind(unexplained)
        .bind(passed)
        .execute(pg)
        .await
        .map(|_| ())
}

fn th() -> serde_json::Value {
    serde_json::json!({"agreement_min": 0.99, "te_max": 0.0001})
}

fn exp() -> String {
    format!("e-{}", uuid::Uuid::new_v4().simple())
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn a_pass_cannot_contradict_the_thresholds_the_same_row_records() {
    let pg = pool().await;
    assert!(insert(&pg, &exp(), 1.0, 0.0, th(), 0, true).await.is_ok());
    assert!(
        insert(&pg, &exp(), 0.95, 0.0, th(), 0, true).await.is_err(),
        "below its own agreement_min"
    );
    assert!(
        insert(&pg, &exp(), 1.0, 0.01, th(), 0, true).await.is_err(),
        "above its own te_max"
    );
    assert!(
        insert(&pg, &exp(), 1.0, 0.0, th(), 1, true).await.is_err(),
        "an unexplained disagreement"
    );
    // A failure may record anything: the table keeps what was found.
    assert!(insert(&pg, &exp(), 0.5, 0.2, th(), 9, false).await.is_ok());
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn thresholds_must_be_complete_and_numeric() {
    let pg = pool().await;
    for t in [
        serde_json::json!({"agreement_min": 0.9}),
        serde_json::json!({"te_max": 0.1}),
        serde_json::json!({}),
        serde_json::json!({"agreement_min": "x", "te_max": 1}),
    ] {
        assert!(
            insert(&pg, &exp(), 1.0, 0.0, t.clone(), 0, true)
                .await
                .is_err(),
            "{t}"
        );
        assert!(
            insert(&pg, &exp(), 0.1, 0.0, t.clone(), 0, false)
                .await
                .is_err(),
            "{t} even for a failure"
        );
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn one_verdict_per_pair_experiment_and_thresholds_and_the_record_is_append_only() {
    let pg = pool().await;
    let e = exp();
    insert(&pg, &e, 1.0, 0.0, th(), 0, true).await.unwrap();
    assert!(
        insert(&pg, &e, 1.0, 0.0, th(), 0, true).await.is_err(),
        "the same inputs, a second row"
    );
    // Other thresholds are another question, so another row.
    insert(
        &pg,
        &e,
        1.0,
        0.0,
        serde_json::json!({"agreement_min": 0.999, "te_max": 0.0001}),
        0,
        true,
    )
    .await
    .unwrap();
    assert!(sqlx::query(
        "UPDATE strategy_reconciliation SET passed = FALSE WHERE experiment_id = $1"
    )
    .bind(&e)
    .execute(&pg)
    .await
    .is_err());
    assert!(
        sqlx::query("DELETE FROM strategy_reconciliation WHERE experiment_id = $1")
            .bind(&e)
            .execute(&pg)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn a_reconciliation_names_two_different_revisions_the_platform_holds() {
    let pg = pool().await;
    let same = sqlx::query(INSERT)
        .bind(A)
        .bind(A)
        .bind(exp())
        .bind(1.0)
        .bind(0.0)
        .bind(th())
        .bind(0)
        .bind(true)
        .execute(&pg)
        .await;
    assert!(same.is_err(), "a layer reconciled against itself");
    let unknown = sqlx::query(INSERT)
        .bind("c".repeat(64))
        .bind(B)
        .bind(exp())
        .bind(1.0)
        .bind(0.0)
        .bind(th())
        .bind(0)
        .bind(true)
        .execute(&pg)
        .await;
    assert!(unknown.is_err(), "a revision nobody holds");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL pointing at a scratch database"]
async fn the_platform_role_can_record_and_read_and_cannot_edit() {
    let pg = pool().await;
    let e = exp();
    let mut tx = pg.begin().await.unwrap();
    tx.execute("SET LOCAL ROLE app_role").await.unwrap();
    sqlx::query(INSERT)
        .bind(A)
        .bind(B)
        .bind(&e)
        .bind(1.0)
        .bind(0.0)
        .bind(th())
        .bind(0)
        .bind(true)
        .execute(&mut *tx)
        .await
        .expect("app_role can insert a verdict");
    let n: i64 =
        sqlx::query("SELECT count(*) FROM strategy_reconciliation WHERE experiment_id = $1")
            .bind(&e)
            .fetch_one(&mut *tx)
            .await
            .expect("app_role can read it")
            .get(0);
    assert_eq!(n, 1);
    tx.rollback().await.unwrap();

    let mut tx = pg.begin().await.unwrap();
    tx.execute("SET LOCAL ROLE app_role").await.unwrap();
    let upd = sqlx::query("UPDATE strategy_reconciliation SET passed = FALSE")
        .execute(&mut *tx)
        .await;
    assert!(upd.is_err(), "app_role has no UPDATE");
    tx.rollback().await.unwrap();
}
