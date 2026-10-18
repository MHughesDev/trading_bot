//! Live-Postgres checks for the reproduction ticket (migration 0241; S-10.8,
//! ADR-S10-18).
//!
//! `cargo test -p ledger --test pg_reproduction -- --ignored --test-threads=1`
//!
//! Everything runs as `app_role`, the role the platform's pool belongs to: an
//! owner-connected test cannot see a missing GRANT or a policy that only binds the
//! role the platform runs as, which is how 0145 and 0147 shipped broken.

use ledger::pg::PgTrialLedger;
use ledger::reproduction::ReproductionRecord;
use ledger::{
    DispatchContext, LedgerError, Registration, TrialEvent, TrialState, TrialSubject,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, PgConnection, PgPool};

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");
const M0241: &str = include_str!("../../../migrations/0241_reproduction.sql");

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/postgres".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

const ROLES: &[&str] = &["app_role", "agent_role", "internal_ml_role", "backtest_role"];

/// A scratch database with the ledger and 0241, and a second pool that is
/// `app_role`. 0044's table grants are restated for the two tables this test
/// reads, because 0044 needs schemas this scratch database does not have.
async fn scratch(name: &str) -> (PgPool, PgPool) {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    for role in ROLES {
        let _ = admin
            .execute(format!("CREATE ROLE {role} NOLOGIN").as_str())
            .await;
    }
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let owner = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    owner.execute(M0043).await.expect("0043 applies");
    owner
        .execute(
            "GRANT USAGE ON SCHEMA mlops TO app_role;
             GRANT SELECT, INSERT ON mlops.trial, mlops.trial_event TO app_role;
             GRANT SELECT ON mlops.trial_state TO app_role;
             DO $$ BEGIN
               EXECUTE 'ALTER TABLE mlops.trial ENABLE ROW LEVEL SECURITY';
               EXECUTE 'ALTER TABLE mlops.trial FORCE ROW LEVEL SECURITY';
               EXECUTE 'ALTER TABLE mlops.trial_event ENABLE ROW LEVEL SECURITY';
               EXECUTE 'ALTER TABLE mlops.trial_event FORCE ROW LEVEL SECURITY';
               EXECUTE $p$CREATE POLICY tenant_isolation ON mlops.trial
                  USING (tenant_id = current_setting('app.tenant_id', true))
                  WITH CHECK (tenant_id = current_setting('app.tenant_id', true))$p$;
               EXECUTE $p$CREATE POLICY tenant_isolation ON mlops.trial_event
                  USING (tenant_id = current_setting('app.tenant_id', true))
                  WITH CHECK (tenant_id = current_setting('app.tenant_id', true))$p$;
             END $$;",
        )
        .await
        .expect("test grants apply");
    owner.execute(M0241).await.expect("0241 applies");
    let app = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                conn.execute("SET ROLE app_role").await?;
                Ok(())
            })
        })
        .connect(&with_db(&base_url(), name))
        .await
        .unwrap();
    (owner, app)
}

fn subject(n: u64) -> TrialSubject {
    TrialSubject {
        config_hash: format!("sha256:cfg{n}"),
        config: serde_json::json!({ "n": n }),
        dataset_id: "slice:sha256:abc".into(),
        split_spec_id: None,
        code_hash: "sha256:code".into(),
        image_digest: "engine@1".into(),
        seed_set: vec![n as i64],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    }
}

fn record(run: &str, outcome: &str) -> ReproductionRecord {
    ReproductionRecord {
        run_id: run.into(),
        outcome: outcome.into(),
        receipt_stored: Some(serde_json::json!({"calendar_version": "1"})),
        receipt_rerun: serde_json::json!({"calendar_version": "1"}),
        differences: serde_json::json!([]),
        was_value: None,
        now_value: None,
        reproduced_by: "test".into(),
    }
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn a_reproduction_runs_as_the_platforms_role_and_spends_no_trial() {
    let (owner, app) = scratch("repro_ticket").await;
    let led = PgTrialLedger::new(app.clone());
    let ctx = DispatchContext::human("tenant-r", "mason", 0.1).with_experiment("exp-r");
    let s = subject(1);

    // A completed look, registered the ordinary way.
    let mut t = led
        .register_async(&Registration::new(&ctx, &s, 1.0))
        .await
        .expect("registered");
    led.transition_async(&mut t, TrialEvent::to(TrialState::Running))
        .await
        .unwrap();
    led.settle_async(t, TrialEvent::completed("run-1", None))
        .await
        .unwrap();
    let trials_before = count(&owner, "mlops.trial").await;
    let events_before = count(&owner, "mlops.trial_event").await;

    let ticket = led
        .begin_reproduction_async("tenant-r", &s.config_hash)
        .await
        .expect("a completed trial can be reproduced as app_role");
    assert!(ticket.reproduces().is_some());
    led.record_reproduction_async(ticket, &record(&s.config_hash, "bit_identical"))
        .await
        .expect("app_role may append a reproduction");

    assert_eq!(count(&owner, "mlops.trial").await, trials_before, "no trial row");
    assert_eq!(count(&owner, "mlops.trial_event").await, events_before, "no event");
    assert_eq!(count(&owner, "mlops.reproduction").await, 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn a_reproduction_row_cannot_be_edited_or_deleted_by_anyone() {
    let (owner, app) = scratch("repro_append_only").await;
    let led = PgTrialLedger::new(app.clone());
    let ctx = DispatchContext::human("tenant-r", "mason", 0.1).with_experiment("exp-r");
    let s = subject(2);
    let mut t = led
        .register_async(&Registration::new(&ctx, &s, 1.0))
        .await
        .unwrap();
    led.transition_async(&mut t, TrialEvent::to(TrialState::Running))
        .await
        .unwrap();
    led.settle_async(t, TrialEvent::completed("run-2", None))
        .await
        .unwrap();
    let ticket = led
        .begin_reproduction_async("tenant-r", &s.config_hash)
        .await
        .unwrap();
    led.record_reproduction_async(ticket, &record(&s.config_hash, "bit_identical"))
        .await
        .unwrap();

    // The platform's role has no UPDATE or DELETE grant at all.
    let mut tx = app.begin().await.unwrap();
    tx.execute("SELECT set_config('app.tenant_id', 'tenant-r', true)")
        .await
        .unwrap();
    assert!(tx
        .execute("UPDATE mlops.reproduction SET outcome = 'drifted'")
        .await
        .is_err());
    drop(tx);
    // And the owner is refused by the trigger: the record is of what was found.
    assert!(owner
        .execute("UPDATE mlops.reproduction SET outcome = 'drifted'")
        .await
        .is_err());
    assert!(owner.execute("DELETE FROM mlops.reproduction").await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn only_a_completed_look_can_be_reproduced_and_the_schema_refuses_a_malformed_row() {
    let (owner, app) = scratch("repro_refusals").await;
    let led = PgTrialLedger::new(app.clone());
    let ctx = DispatchContext::human("tenant-r", "mason", 0.1).with_experiment("exp-r");
    let s = subject(3);
    // Registered, never finished.
    led.register_async(&Registration::new(&ctx, &s, 1.0))
        .await
        .unwrap();
    assert!(matches!(
        led.begin_reproduction_async("tenant-r", &s.config_hash).await,
        Err(LedgerError::NothingToReproduce(_))
    ));
    // Another tenant's look is not reproducible by this one.
    assert!(matches!(
        led.begin_reproduction_async("tenant-other", &s.config_hash)
            .await,
        Err(LedgerError::NothingToReproduce(_))
    ));
    // The CHECKs hold where a second writer cannot route around the Rust.
    let trial: uuid::Uuid = sqlx::query_scalar("SELECT trial_id FROM mlops.trial LIMIT 1")
        .fetch_one(&owner)
        .await
        .unwrap();
    for outcome in ["close_enough", "drifted", "receipt_changed"] {
        // `close_enough` is outside the closed list; `drifted` names no numbers;
        // `receipt_changed` names no difference.
        let res = sqlx::query(
            "INSERT INTO mlops.reproduction
               (tenant_id, original_trial_id, run_id, outcome, receipt_rerun, receipt_missing,
                differences, was_value, now_value, reproduced_by)
             VALUES ('t', $1, 'r', $2, '{}'::jsonb, false, '[]'::jsonb, NULL, NULL, 'x')",
        )
        .bind(trial)
        .bind(outcome)
        .execute(&owner)
        .await;
        assert!(res.is_err(), "`{outcome}` with nothing behind it should be refused by a CHECK");
    }
}
