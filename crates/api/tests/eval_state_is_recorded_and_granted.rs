//! Calibration rows and holdout draws are written, refuse what they should, and
//! are reachable by the platform's own role (S-4.12, S-4.17; migration 0177).
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test eval_state_is_recorded_and_granted -- --ignored --test-threads=1
//! ```
//!
//! `eval_store.rs` is compiled in through `#[path]` so this test exercises the
//! module even before `lib.rs` declares it.

#[path = "../src/eval_store.rs"]
mod eval_store;

use harness::eval::cases;
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

async fn db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS eval_state_test WITH (FORCE)")
        .await
        .unwrap();
    admin.execute("CREATE DATABASE eval_state_test").await.unwrap();
    // The role is cluster-wide; the platform's migrations create it, a scratch
    // cluster may not have it.
    let _ = admin
        .execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='app_role') \
             THEN CREATE ROLE app_role NOLOGIN; END IF; END $$",
        )
        .await;
    let pool = PgPool::connect(&with_db(&base_url(), "eval_state_test"))
        .await
        .unwrap();
    // The one column set the calibration reads (0062), without the rest of
    // agent_runs.
    pool.execute(
        "CREATE TABLE agent_runs (id BIGSERIAL PRIMARY KEY, model TEXT, \
           prompt_tokens BIGINT, est_prompt_tokens BIGINT, \
           created_at TIMESTAMPTZ NOT NULL DEFAULT now())",
    )
    .await
    .unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("two below the root")
        .join("migrations/0177_eval_calibration_and_holdout.sql");
    let sql = std::fs::read_to_string(path).expect("0177 readable");
    pool.execute(sql.as_str()).await.expect("0177 applies");
    pool
}

async fn seed_runs(pool: &PgPool, model: &str, n: i64) {
    for i in 0..n {
        sqlx::query(
            "INSERT INTO agent_runs (model, est_prompt_tokens, prompt_tokens) VALUES ($1,1000,$2)",
        )
        .bind(model)
        .bind(800 + i * 8)
        .execute(pool)
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_row_exists_per_model_with_ten_runs_and_none_below() {
    let pool = db().await;
    seed_runs(&pool, "model-a", 50).await;
    seed_runs(&pool, "model-b", 9).await;

    let written = eval_store::refresh_calibration(&pool).await.unwrap();
    let names: Vec<&str> = written.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(names, vec!["model-a"], "nine runs is not enough");

    let a = eval_store::bytes_per_token(&pool, "model-a").await.unwrap();
    assert!((2.5..=5.0).contains(&a));
    // An unseen model keeps the prior.
    assert!((eval_store::bytes_per_token(&pool, "model-b").await.unwrap() - 4.0).abs() < 1e-9);
    // The budget suite's number.
    let rate = eval_store::worst_over_estimate_rate(&pool).await.unwrap().unwrap();
    assert!(rate >= 0.95, "over-estimate rate {rate}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_redraw_is_recorded_with_its_requester_and_refused_before_growth() {
    let pool = db().await;
    let h = cases::standard_holdout();
    eval_store::record_holdout_draw(&pool, &h, "harness-test").await.unwrap();

    let (requested_by, version, n): (String, String, i32) = sqlx::query_as(
        "SELECT requested_by, harness_version, array_length(sealed_ids,1) FROM eval_holdout_draw",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(requested_by, h.draw.requested_by);
    assert_eq!(version, "harness-test");
    assert_eq!(n as usize, h.sealed.len());

    // A new salt over the same corpus: refused, with the reason.
    let mut again = h.clone();
    again.draw.salt = "another-salt".into();
    let err = eval_store::record_holdout_draw(&pool, &again, "harness-test-2")
        .await
        .expect_err("no growth, no re-draw");
    assert!(matches!(err, eval_store::EvalStoreError::RedrawRefused(_)), "{err}");

    // The record cannot be edited.
    let e = sqlx::query("UPDATE eval_holdout_draw SET requested_by = 'someone else'")
        .execute(&pool)
        .await
        .expect_err("append-only");
    assert!(e.to_string().contains("append-only"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_losing_policy_is_recorded_with_its_weights_and_corpus_and_cannot_be_edited() {
    use harness::context::fit::{corpus_fingerprint, PromotionRecord};
    let pool = db().await;
    let rec = PromotionRecord {
        policy_version: "v2".into(),
        weights: harness::context::policy::Weights::default(),
        corpus_size: 0,
        corpus_fingerprint: corpus_fingerprint(&[]),
        promoted: false,
        refused: vec!["qwen2.5-7b-instruct lost utility: 60% -> 40%".into()],
        report: vec!["x".into()],
    };
    eval_store::record_promotion(&pool, &rec, "harness-test").await.unwrap();
    let (promoted, w): (bool, serde_json::Value) =
        sqlx::query_as("SELECT promoted, weights_json FROM context_policy_promotion")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!promoted);
    assert!(w.is_object(), "the weights travel with the loss");
    // A refusal that claims to be a promotion is refused by the schema.
    let mut lie = rec.clone();
    lie.promoted = true;
    assert!(eval_store::record_promotion(&pool, &lie, "harness-test").await.is_err());
    let e = sqlx::query("DELETE FROM context_policy_promotion")
        .execute(&pool)
        .await
        .expect_err("append-only");
    assert!(e.to_string().contains("append-only"), "{e}");
}

/// Owner-connected tests cannot see a missing GRANT; this one switches role.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_platform_role_can_use_both_tables() {
    let pool = db().await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO agent_token_calibration (model, bytes_per_token, sample_n, over_estimate_rate) \
         VALUES ('m', 3.5, 10, 0.97)",
    )
    .execute(&mut *tx)
    .await
    .expect("app_role may insert a calibration");
    sqlx::query("UPDATE agent_token_calibration SET sample_n = 11")
        .execute(&mut *tx)
        .await
        .expect("app_role may update a calibration");
    sqlx::query(
        "INSERT INTO context_policy_promotion (policy_version, weights_json, corpus_size,          corpus_fingerprint, promoted, harness_version) VALUES ('v2','{}',0,'f',true,'t')",
    )
    .execute(&mut *tx)
    .await
    .expect("app_role may record a promotion");
    sqlx::query(
        "INSERT INTO eval_holdout_draw (salt, requested_by, harness_version, drawn_on, corpus_size, sealed_ids) \
         VALUES ('s','r','v', current_date, 5, ARRAY['a'])",
    )
    .execute(&mut *tx)
    .await
    .expect("app_role may record a draw");
    let n: (i64,) = sqlx::query_as("SELECT count(*) FROM eval_holdout_draw")
        .fetch_one(&mut *tx)
        .await
        .expect("app_role may read draws");
    assert_eq!(n.0, 1);
    tx.rollback().await.unwrap();
}

/// The standing draw has a caller: it is recorded once, and asking again is a
/// no-op rather than a refusal or a duplicate row (S-4.17).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_standing_draw_is_recorded_once_and_asking_again_changes_nothing() {
    let pool = db().await;
    assert!(eval_store::record_standing_draw(&pool).await.unwrap(), "first call writes");
    assert!(!eval_store::record_standing_draw(&pool).await.unwrap(), "second call is a no-op");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM eval_holdout_draw")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}
