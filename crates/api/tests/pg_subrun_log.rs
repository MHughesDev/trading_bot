//! A sub-run's own effect log, and the Delegation on both sides (Set S S-2.12),
//! plus the claim path of the lease against a migrated Postgres.
//!
//! The lease split, the claim and the settle had unit tests and a scratch-database
//! check of the migration alone; nothing drove `claim` against the tables. This does,
//! end to end through the same functions the campaign driver and the worker call.
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/s_runtime_scratch \
//!   cargo test -j 2 -p api --test pg_subrun_log -- --ignored --test-threads=1
//! ```

use api::agent::subrun::{self, PinnedBrief};
use harness::trace::Event;
use serde_json::{json, Value};
use sqlx::{Connection, PgPool};
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/s_runtime_scratch".into())
}

async fn pool() -> PgPool {
    PgPool::connect(&url())
        .await
        .expect("the scratch database is up")
}

/// A parent run with a recorded budget cap (a `RunSpec` row), and a campaign
/// defined by it with a pinned brief.
async fn campaign(pg: &PgPool, cap_tokens: u64) -> (Uuid, Uuid, Uuid, subrun::CampaignBrief) {
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("test-fixture-subrun-{user}@test.invalid"))
        .execute(pg)
        .await
        .unwrap();
    let parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model)
         VALUES ($1, $2, 'waiting_backtest', 'test-fixture:subrun', 'anthropic', 'claude-opus-5')",
    )
    .bind(parent)
    .bind(user)
    .execute(pg)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version)
         VALUES ($1, 1, 'state', 'RunSpec', $2, 'test')",
    )
    .bind(parent)
    .bind(json!({
        "profile": { "budgets": { "max_tokens_per_task": cap_tokens },
                     "orchestration": { "max_steps": 40 } },
        "max_trials": 20
    }))
    .execute(pg)
    .await
    .unwrap();

    let prereg = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_lifecycle.preregistration
             (prereg_id, run_id, objective, benchmark, delta_practical, window_start, window_end,
              embargo_bars, cost_model_version, null_family, trial_budget, gate_profile,
              primary_hypothesis, falsifier, prereg_hash)
         VALUES ($1, $2, '{}', '{}', 0.1, '2024-01-01', '2025-01-01', 24, 'v1', 'iid', 20, 'std',
                 'momentum survives costs', 'sharpe below 0.3 net', 'h-test')",
    )
    .bind(prereg)
    .bind(parent)
    .execute(pg)
    .await
    .unwrap();
    let campaign_id = Uuid::new_v4();
    let brief = subrun::record_campaign(
        pg,
        campaign_id,
        parent,
        &user.to_string(),
        prereg,
        "h-test",
        &format!("job-{campaign_id}"),
        &PinnedBrief {
            text: "the evidence, and nothing of how the parent argued".into(),
            projection: subrun::worker_projection_name().into(),
            shown: vec!["r1".into()],
            withheld: 2,
        },
        subrun::MAX_SUBAGENT_DEPTH,
    )
    .await
    .expect("the campaign row is written");
    (user, parent, campaign_id, brief)
}

#[tokio::test]
#[ignore = "needs DATABASE_URL: a migrated scratch database"]
async fn a_claim_leases_from_the_parent_and_records_the_delegation_on_both_sides() {
    let pg = pool().await;
    let (user, parent, campaign_id, brief) = campaign(&pg, 400_000).await;

    let claim = subrun::claim(
        &pg,
        campaign_id,
        parent,
        &user.to_string(),
        "hypothesize",
        1,
        &brief,
        1,
    )
    .await
    .expect("a funded parent can claim");
    assert!(claim.fresh);
    let lease = claim
        .lease
        .as_ref()
        .expect("a parent with a cap hands out a lease");
    // A quarter of the remaining 400k, under the 100k ceiling.
    assert_eq!(lease.tokens(), 100_000);
    let persisted = subrun::lease_of(&pg, claim.sub_run_id).await.unwrap();
    assert_eq!(persisted.tokens, Some(100_000));

    // The same position claimed again hands out nothing and writes no second pair.
    let again = subrun::claim(
        &pg,
        campaign_id,
        parent,
        &user.to_string(),
        "hypothesize",
        1,
        &brief,
        1,
    )
    .await
    .unwrap();
    assert!(!again.fresh);
    assert_eq!(again.sub_run_id, claim.sub_run_id);
    assert!(again.lease.is_none());

    // Parent side: a Delegation state row in the parent's own effect log, and it
    // projects to a trace event.
    let parent_row: Value = sqlx::query_scalar(
        "SELECT payload_json FROM agent_effect_log
          WHERE run_id = $1 AND direction = 'state' AND kind = 'Delegation'",
    )
    .bind(parent)
    .fetch_one(&pg)
    .await
    .expect("exactly one parent-side Delegation row");
    assert_eq!(parent_row["child_trace_id"], json!(claim.sub_run_id));
    assert_eq!(parent_row["brief_digest"], json!(brief.digest));

    // Child side: the first row of the sub-run's own log, same digest, names the parent.
    let child_row: Value = sqlx::query_scalar(
        "SELECT payload_json FROM agent_lifecycle.sub_run_effect
          WHERE sub_run_id = $1 AND seq = 1 AND kind = 'Delegation'",
    )
    .bind(claim.sub_run_id)
    .fetch_one(&pg)
    .await
    .expect("the child's log opens with its Delegation");
    assert_eq!(
        child_row["brief_digest"], parent_row["brief_digest"],
        "the two sides agree"
    );
    assert_eq!(child_row["parent_run_id"], json!(parent));

    let spans = api::trace_from_effects::spans_for_run(&pg, parent)
        .await
        .unwrap();
    assert!(
        spans
            .iter()
            .any(|s| matches!(&s.event, Event::Delegation { child_trace_id, .. } if *child_trace_id == claim.sub_run_id.to_string())),
        "the trace projection does not show the delegation: {spans:?}"
    );

    // The worker's rows follow, in order, and the sequence does not collide.
    subrun::log_effect(
        &pg,
        claim.sub_run_id,
        "effect",
        "CallModel",
        &json!({ "attempt": 1 }),
    )
    .await;
    subrun::log_effect(
        &pg,
        claim.sub_run_id,
        "input",
        "ModelReply",
        &json!({ "raw": "{}" }),
    )
    .await;
    let kinds: Vec<(i32, String)> = sqlx::query_as(
        "SELECT seq, kind FROM agent_lifecycle.sub_run_effect WHERE sub_run_id = $1 ORDER BY seq",
    )
    .bind(claim.sub_run_id)
    .fetch_all(&pg)
    .await
    .unwrap();
    assert_eq!(
        kinds,
        vec![
            (1, "Delegation".into()),
            (2, "CallModel".into()),
            (3, "ModelReply".into())
        ]
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL: a migrated scratch database"]
async fn leases_handed_out_never_exceed_what_the_parent_had() {
    let pg = pool().await;
    // 100k remaining: the first worker is leased 25k, the next a quarter of 75k, ...
    let (user, parent, campaign_id, brief) = campaign(&pg, 100_000).await;
    let mut total = 0u64;
    for seq in 1..=6 {
        let c = subrun::claim(
            &pg,
            campaign_id,
            parent,
            &user.to_string(),
            "hypothesize",
            seq,
            &brief,
            1,
        )
        .await
        .expect("claim");
        total += c.lease.map_or(0, |l| l.tokens());
    }
    assert!(total <= 100_000, "{total} tokens leased out of 100000");
    assert!(
        total > 50_000,
        "the leases should have used most of the parent's budget: {total}"
    );
    let summed: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(lease_tokens), 0)::BIGINT FROM agent_lifecycle.sub_run WHERE parent_run_id = $1",
    )
    .bind(parent)
    .fetch_one(&pg)
    .await
    .unwrap();
    assert_eq!(
        u64::try_from(summed).unwrap(),
        total,
        "the persisted leases are the leases"
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL: a migrated scratch database"]
async fn a_parent_with_no_recorded_cap_hands_out_no_lease_and_never_a_zero() {
    let pg = pool().await;
    let (user, parent, campaign_id, brief) = campaign(&pg, 1_000).await;
    // Remove the cap's evidence by claiming against a parent that has no RunSpec.
    let orphan = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model)
         VALUES ($1, $2, 'running', 'test-fixture:nospec', 'anthropic', 'claude-opus-5')",
    )
    .bind(orphan)
    .bind(user)
    .execute(&pg)
    .await
    .unwrap();
    let _ = parent;
    let c = subrun::claim(
        &pg,
        campaign_id,
        orphan,
        &user.to_string(),
        "hypothesize",
        9,
        &brief,
        1,
    )
    .await
    .unwrap();
    assert!(c.lease.is_none());
    let (t, s): (Option<i64>, Option<i32>) = sqlx::query_as(
        "SELECT lease_tokens, lease_steps FROM agent_lifecycle.sub_run WHERE sub_run_id = $1",
    )
    .bind(c.sub_run_id)
    .fetch_one(&pg)
    .await
    .unwrap();
    assert_eq!((t, s), (None, None), "a missing lease is NULL, not zero");
}

/// Owner-connected tests cannot see a missing GRANT (0145/0147 were both found
/// live), so this runs as the application role.
#[tokio::test]
#[ignore = "needs DATABASE_URL: a migrated scratch database"]
async fn the_sub_run_log_is_append_only_and_granted_to_the_application_role() {
    let pg = pool().await;
    let (user, parent, campaign_id, brief) = campaign(&pg, 400_000).await;
    let claim = subrun::claim(
        &pg,
        campaign_id,
        parent,
        &user.to_string(),
        "hypothesize",
        1,
        &brief,
        1,
    )
    .await
    .unwrap();

    let mut conn = sqlx::PgConnection::connect(&url()).await.unwrap();
    let mut tx = conn.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    // It can read and append.
    sqlx::query(
        "INSERT INTO agent_lifecycle.sub_run_effect (sub_run_id, seq, direction, kind, payload_json, harness_version)
         VALUES ($1, 50, 'effect', 'CallModel', '{}', 'test')",
    )
    .bind(claim.sub_run_id)
    .execute(&mut *tx)
    .await
    .expect("app_role may append to the sub-run log");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_lifecycle.sub_run_effect WHERE sub_run_id = $1",
    )
    .bind(claim.sub_run_id)
    .fetch_one(&mut *tx)
    .await
    .expect("app_role may read it");
    assert!(n >= 2);
    // It cannot rewrite or remove (no grant), and neither can the owner (trigger).
    sqlx::query("SAVEPOINT s1").execute(&mut *tx).await.unwrap();
    assert!(sqlx::query(
        "UPDATE agent_lifecycle.sub_run_effect SET kind = 'x' WHERE sub_run_id = $1"
    )
    .bind(claim.sub_run_id)
    .execute(&mut *tx)
    .await
    .is_err());
    sqlx::query("ROLLBACK TO SAVEPOINT s1")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query("DELETE FROM agent_lifecycle.sub_run_effect WHERE sub_run_id = $1")
            .bind(claim.sub_run_id)
            .execute(&mut *tx)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();

    let mut tx = conn.begin().await.unwrap();
    let owner_update =
        sqlx::query("UPDATE agent_lifecycle.sub_run_effect SET kind = 'x' WHERE sub_run_id = $1")
            .bind(claim.sub_run_id)
            .execute(&mut *tx)
            .await;
    assert!(
        owner_update.is_err(),
        "the owner is held by the trigger, not only by the grant"
    );
}
