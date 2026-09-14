//! The sealed holdout against live Postgres (SPEC §12.7, AT-33). Ignored by default:
//! `cargo test -p ledger --test pg_holdout -- --ignored --test-threads=1`.

use ledger::holdout::HoldoutClaim;
use ledger::pg::PgTrialLedger;
use ledger::{DispatchContext, Registration, TrialSubject};
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn one_evaluation_per_lineage_ever_and_every_request_is_logged() {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
    admin.execute("DROP DATABASE IF EXISTS ledger_holdout WITH (FORCE)").await.unwrap();
    admin.execute("CREATE DATABASE ledger_holdout").await.unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "ledger_holdout")).await.unwrap();
    pool.execute(include_str!("../../../migrations/0043_trial_ledger.sql")).await.unwrap();
    let ledger = PgTrialLedger::new(pool.clone());

    let ctx = DispatchContext::human("tenant-h", "m", 0.1).with_experiment("e");
    let subject = TrialSubject {
        config_hash: "sha256:vault".into(),
        config: serde_json::json!({}),
        dataset_id: "d".into(),
        split_spec_id: None,
        code_hash: "c".into(),
        image_digest: "i".into(),
        seed_set: vec![1],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    };
    let ticket = ledger.register_async(&Registration::new(&ctx, &subject, 1.0)).await.unwrap();

    assert_eq!(ledger.claim_sealed_holdout_async("tenant-h", "ema-family", "alice").await.unwrap(), HoldoutClaim::First);
    // A concurrent second "first" request cannot reach the data.
    let racing = ledger.claim_sealed_holdout_async("tenant-h", "ema-family", "mallory").await.unwrap_err();
    assert!(racing.to_string().contains("already been claimed"), "{racing}");

    let result = serde_json::json!({ "sharpe": 0.8 });
    ledger.record_sealed_holdout_async("tenant-h", "ema-family", ticket.trial_id(), &result).await.unwrap();
    assert!(ledger.record_sealed_holdout_async("tenant-h", "ema-family", ticket.trial_id(), &result).await.is_err(), "one result, ever");

    match ledger.claim_sealed_holdout_async("tenant-h", "ema-family", "bob").await.unwrap() {
        HoldoutClaim::Repeat { first_trial, result: served, .. } => {
            assert_eq!(first_trial, ticket.trial_id());
            assert_eq!(served, result, "the first result, not a new evaluation");
        }
        HoldoutClaim::First => panic!("a second evaluation was granted"),
    }
    // Other lineages and other tenants are unaffected.
    assert_eq!(ledger.claim_sealed_holdout_async("tenant-h", "rsi-family", "alice").await.unwrap(), HoldoutClaim::First);

    let logged: Vec<(String, bool)> = sqlx::query_as(
        "SELECT requested_by, served_first_result FROM mlops.sealed_holdout_attempt WHERE strategy_lineage_id = 'ema-family' ORDER BY attempted_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(logged, vec![("alice".to_string(), false), ("bob".to_string(), true)]);
    assert!(pool.execute("DELETE FROM mlops.sealed_holdout_attempt").await.is_err());
    assert!(pool.execute("UPDATE mlops.sealed_holdout_call SET result = '{}'").await.is_err());
    pool.close().await;
}
