//! Live-Postgres tests for the deployment record, checkpoints and the decision
//! journal (Set U U-4.2, U-4.4, U-4.5; migrations 0142–0144).
//!
//! Ignored without a database:
//!
//! ```text
//! DATABASE_URL=postgres://… cargo test -j 2 -p storage --test pg_strategy_deployment -- --ignored --test-threads=1
//! ```

use chrono::{Duration, Utc};
use rust_decimal::Decimal;
use storage::postgres::{decision_journal as journal, deployments};
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    sqlx::PgPool::connect(&url).await.expect("connects")
}

fn deployment(user: &str) -> deployments::NewDeployment {
    deployments::NewDeployment {
        deployment_id: Uuid::new_v4(),
        user_id: user.into(),
        strategy_name: "ema_v3".into(),
        revision_hash: "a".repeat(64),
        definition_json: serde_json::json!({"format": "tbot.strategy/3", "name": "ema_v3"}),
        instrument_id: "BTC-USD".into(),
        starting_capital: Decimal::from(25_000),
        model_pins: serde_json::json!([]),
        started_by: user.into(),
        forward_deployment_id: None,
    }
}

fn record(
    deployment_id: Uuid,
    seq: i64,
    decided_at: chrono::DateTime<Utc>,
) -> journal::JournalRecord {
    journal::JournalRecord {
        deployment_id,
        seq,
        tick: seq,
        decided_at,
        cause: "bar_close".into(),
        digest: "b".repeat(64),
        input_digest: "c".repeat(64),
        inputs: serde_json::json!({"event": "bar"}),
        trace: serde_json::json!({"tick": seq}),
        gate_verdicts: serde_json::json!([]),
        missing_reasons: serde_json::json!({}),
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn a_deployment_is_recorded_at_start_and_closed_once() {
    let pool = pool().await;
    let user = format!("u-{}", Uuid::new_v4());
    let d = deployment(&user);
    deployments::start(&pool, &d).await.unwrap();

    let running = deployments::running(&pool).await.unwrap();
    let row = running
        .iter()
        .find(|r| r.deployment_id == d.deployment_id)
        .expect("running");
    assert_eq!(row.environment, "paper");
    assert_eq!(row.venue, "platform_paper");
    assert_eq!(
        deployments::starting_capital(&pool, d.deployment_id)
            .await
            .unwrap(),
        Some(Decimal::from(25_000)),
        "the starting capital is on the record"
    );

    // The same binding twice while running is refused by the database itself.
    let mut dup = deployment(&user);
    dup.deployment_id = Uuid::new_v4();
    assert!(deployments::start(&pool, &dup).await.is_err());

    assert!(deployments::stop(&pool, d.deployment_id, "stopped by user")
        .await
        .unwrap());
    assert!(
        !deployments::stop(&pool, d.deployment_id, "again")
            .await
            .unwrap(),
        "closed once"
    );
    assert!(deployments::running(&pool)
        .await
        .unwrap()
        .iter()
        .all(|r| r.deployment_id != d.deployment_id));
    // Fixed at start: nothing but stopping may change a row.
    let edit =
        sqlx::query("UPDATE strategy_deployment SET starting_capital = 1 WHERE deployment_id = $1")
            .bind(d.deployment_id)
            .execute(&pool)
            .await;
    assert!(edit.is_err());
    // After a stop, the binding can start again.
    deployments::start(&pool, &dup).await.unwrap();
    deployments::stop(&pool, dup.deployment_id, "test")
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn the_latest_checkpoint_wins() {
    let pool = pool().await;
    let d = deployment(&format!("u-{}", Uuid::new_v4()));
    deployments::start(&pool, &d).await.unwrap();
    assert!(deployments::load_checkpoint(&pool, d.deployment_id)
        .await
        .unwrap()
        .is_none());
    for tick in [3_i64, 4] {
        deployments::save_checkpoint(
            &pool,
            d.deployment_id,
            &"a".repeat(64),
            tick,
            &serde_json::json!({"tick": tick}),
        )
        .await
        .unwrap();
    }
    let cp = deployments::load_checkpoint(&pool, d.deployment_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cp.tick, 4);
    assert_eq!(cp.state["tick"], 4);
    deployments::stop(&pool, d.deployment_id, "test")
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL"]
async fn the_journal_is_append_only_and_pruned_by_age() {
    let pool = pool().await;
    let d = deployment(&format!("u-{}", Uuid::new_v4()));
    deployments::start(&pool, &d).await.unwrap();
    let old = Utc::now() - Duration::days(200);
    journal::append(&pool, &record(d.deployment_id, 1, old))
        .await
        .unwrap();
    journal::append(&pool, &record(d.deployment_id, 2, Utc::now()))
        .await
        .unwrap();
    assert_eq!(journal::last_seq(&pool, d.deployment_id).await.unwrap(), 2);
    // One record per decision: a second record with the same seq is refused.
    assert!(
        journal::append(&pool, &record(d.deployment_id, 2, Utc::now()))
            .await
            .is_err()
    );
    // Append-only.
    let edit =
        sqlx::query("UPDATE strategy_decision_journal SET cause = 'fill' WHERE deployment_id = $1")
            .bind(d.deployment_id)
            .execute(&pool)
            .await;
    assert!(edit.is_err());
    // Retention removes the old one only.
    journal::prune_older_than(
        &pool,
        Utc::now() - Duration::days(journal::DEFAULT_RETENTION_DAYS),
    )
    .await
    .unwrap();
    let left = journal::for_deployment(&pool, d.deployment_id)
        .await
        .unwrap();
    assert_eq!(left.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![2]);
    deployments::stop(&pool, d.deployment_id, "test")
        .await
        .unwrap();
}
