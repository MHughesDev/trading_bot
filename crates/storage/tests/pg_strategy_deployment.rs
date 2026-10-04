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
        target: None,
        parent_deployment_id: None,
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

/// Set U U-6.4: a selection-bound deployment is a parent row with its target,
/// its members are child rows, and every allocation change is journaled —
/// append-only.
#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0148"]
async fn a_selection_bound_deployment_records_its_members_and_every_change() {
    let pg = pool().await;
    let user = format!("u-{}", Uuid::new_v4());
    let selection = "b".repeat(64);
    let mut parent = deployment(&user);
    parent.instrument_id = format!("selection:{selection}");
    parent.target =
        Some(serde_json::json!({"kind": "selection", "selection": selection, "top_n": 2}));
    deployments::start(&pg, &parent)
        .await
        .expect("parent recorded");

    // A target whose instrument_id does not name its selection is refused.
    let mut wrong = deployment(&user);
    wrong.strategy_name = "other".into();
    wrong.target =
        Some(serde_json::json!({"kind": "selection", "selection": selection, "top_n": 2}));
    assert!(deployments::start(&pg, &wrong).await.is_err());

    let mut child = deployment(&user);
    child.instrument_id = "ETH-USD".into();
    child.starting_capital = Decimal::new(12_500, 0);
    child.parent_deployment_id = Some(parent.deployment_id);
    deployments::start(&pg, &child)
        .await
        .expect("member recorded");
    let kids = deployments::running_children(&pg, parent.deployment_id)
        .await
        .unwrap();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].parent_deployment_id, Some(parent.deployment_id));

    let event =
        |change: &str, weight: Decimal, action: Option<&str>| deployments::AllocationEvent {
            deployment_id: parent.deployment_id,
            at: Utc::now(),
            selection_run_id: None,
            change: change.into(),
            instrument_id: "ETH-USD".into(),
            member_deployment_id: Some(child.deployment_id),
            weight,
            capital: weight * parent.starting_capital,
            action: action.map(str::to_string),
        };
    let half = Decimal::new(5, 1);
    assert_eq!(
        deployments::record_allocation(&pg, &event("entered", half, None))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        deployments::record_allocation(&pg, &event("left", Decimal::ZERO, Some("close")))
            .await
            .unwrap(),
        2
    );
    // A leaver must say how it leaves.
    assert!(
        deployments::record_allocation(&pg, &event("left", Decimal::ZERO, None))
            .await
            .is_err()
    );
    let latest = deployments::latest_allocation(&pg, parent.deployment_id)
        .await
        .unwrap();
    assert_eq!(
        latest,
        [(
            "ETH-USD".to_string(),
            Decimal::ZERO,
            Decimal::ZERO,
            "left".to_string()
        )]
    );
    let rewrite =
        sqlx::query("UPDATE deployment_allocation_event SET weight = 1 WHERE deployment_id = $1")
            .bind(parent.deployment_id)
            .execute(&pg)
            .await;
    assert!(rewrite.is_err(), "append-only");

    deployments::stop(&pg, child.deployment_id, "left the selection")
        .await
        .unwrap();
    deployments::stop(&pg, parent.deployment_id, "test")
        .await
        .unwrap();
}

fn verdict(deployment_id: Uuid, verdict: &str) -> journal::ParityCheck {
    let now = Utc::now();
    journal::ParityCheck {
        deployment_id,
        revision: "a".repeat(64),
        window_start: now - Duration::days(1),
        window_end: now,
        first_seq: 1,
        last_seq: 3,
        verdict: verdict.into(),
        decisions: None,
        diverged_seq: None,
        field: None,
        recorded: None,
        replayed: None,
        reason: None,
    }
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0149"]
async fn parity_verdicts_are_recorded_append_only_and_by_the_platform_role() {
    let pool = pool().await;
    let user = format!("u-{}", Uuid::new_v4());
    let d = deployment(&user);
    deployments::start(&pool, &d).await.unwrap();
    let now = Utc::now();
    journal::append(&pool, &record(d.deployment_id, 1, now - Duration::hours(3)))
        .await
        .unwrap();
    let listed = journal::decided_between(&pool, now - Duration::days(1), now)
        .await
        .unwrap();
    assert!(listed.contains(&d.deployment_id));
    let later = journal::decided_between(&pool, now, now + Duration::days(1))
        .await
        .unwrap();
    assert!(!later.contains(&d.deployment_id));

    // Each verdict carries what makes it readable, or the database refuses it.
    let mut ok = verdict(d.deployment_id, "identical");
    ok.decisions = Some(3);
    let id = journal::record_parity(&pool, &ok).await.unwrap();
    assert!(journal::record_parity(&pool, &verdict(d.deployment_id, "identical"))
        .await
        .is_err(), "an identical verdict says how many decisions");
    assert!(journal::record_parity(&pool, &verdict(d.deployment_id, "diverged"))
        .await
        .is_err(), "a divergence names its record and field");
    let mut div = verdict(d.deployment_id, "diverged");
    div.diverged_seq = Some(2);
    div.field = Some("values.f".into());
    div.recorded = Some(serde_json::json!(1.0));
    div.replayed = Some(serde_json::json!(1.5));
    journal::record_parity(&pool, &div).await.unwrap();
    assert!(journal::record_parity(&pool, &verdict(d.deployment_id, "unreplayable"))
        .await
        .is_err(), "an unreplayable verdict says why");

    // Append-only.
    assert!(
        sqlx::query("UPDATE strategy_parity_check SET verdict = 'identical' WHERE check_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM strategy_parity_check WHERE check_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err()
    );

    // The platform runs as app_role, not as the owner these tests connect as.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    let (n,): (i64,) = sqlx::query_as(
        "INSERT INTO strategy_parity_check (deployment_id, revision, window_start, window_end, \
         first_seq, last_seq, verdict, decisions) \
         VALUES ($1, 'r', now() - interval '1 day', now(), 1, 1, 'identical', 1) RETURNING check_id",
    )
    .bind(d.deployment_id)
    .fetch_one(&mut *tx)
    .await
    .expect("app_role records a verdict");
    assert!(n > 0);
    sqlx::query("SELECT count(*) FROM strategy_decision_journal")
        .execute(&mut *tx)
        .await
        .expect("app_role reads the journal it replays");
    tx.rollback().await.unwrap();
}
