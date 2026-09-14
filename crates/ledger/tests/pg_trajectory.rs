//! Agent trajectory rows (SPEC §14.6). Ignored by default:
//! `cargo test -p ledger --test pg_trajectory -- --ignored --test-threads=1`.

use ledger::pg::PgTrialLedger;
use ledger::trajectory::TrajectoryStep;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn steps_append_once_and_labels_append_beside_them() {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.unwrap();
    admin.execute("DROP DATABASE IF EXISTS ledger_traj WITH (FORCE)").await.unwrap();
    admin.execute("CREATE DATABASE ledger_traj").await.unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "ledger_traj")).await.unwrap();
    pool.execute(include_str!("../../../migrations/0043_trial_ledger.sql")).await.unwrap();
    let l = PgTrialLedger::new(pool.clone());

    let traj = Uuid::new_v4();
    let big = serde_json::json!({ "bars": "x".repeat(64 * 1024) });
    let step = TrajectoryStep {
        traj_id: traj,
        step_idx: 0,
        campaign_id: None,
        tool_name: "get_bars".into(),
        tool_schema_hash: format!("sha256:{}", "b".repeat(64)),
        tool_semver: "0.1.0".into(),
        arguments: Some(serde_json::json!({ "instrument_id": "BTC-USD" })),
        result_summary: Some(big),
        error: None,
        latency_ms: Some(12),
        tokens_in: Some(100),
        tokens_out: Some(20),
        propensity: None,
        exploration_flag: None,
        outcome_trial_id: None,
    };
    l.record_trajectory_step_async("t", &step).await.unwrap();
    assert!(l.record_trajectory_step_async("t", &step).await.is_err(), "a step is recorded once");
    let stored: serde_json::Value = sqlx::query_scalar("SELECT result_summary FROM mlops.agent_trajectory WHERE traj_id = $1")
        .bind(traj)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored["truncated"], true, "oversized results are summarized, and say so");

    l.label_trajectory_step_async("t", traj, 0, "unnecessary", "critic@v0").await.unwrap();
    assert!(l.label_trajectory_step_async("t", traj, 0, "brilliant", "critic@v0").await.is_err());
    assert!(l.label_trajectory_step_async("t", traj, 7, "good", "critic@v0").await.is_err(), "labels need a step");
    let err = pool.execute("UPDATE mlops.agent_trajectory SET critic_label = 'good'").await.unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
    pool.close().await;
}
