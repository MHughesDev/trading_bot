//! Live-Postgres checks for the ledger. Ignored by default; run with
//! `cargo test -p ledger --test pg_ledger -- --ignored --test-threads=1`.
//! Each test creates and drops its own scratch database.

use ledger::pg::PgTrialLedger;
use ledger::state::ALL_STATES;
use ledger::{
    verify_events, verify_trials, Decision, DecisionKind, DecisionTier, DispatchContext, OutcomeVector, Registration,
    TerminalReason, TrialEvent, TrialState, TrialSubject,
};
use sqlx::{Connection, Executor, PgConnection, PgPool};

fn base_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

pub async fn scratch(name: &str, migrations: &[&str]) -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres")).await.expect("postgres reachable");
    admin.execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str()).await.unwrap();
    admin.execute(format!("CREATE DATABASE {name}").as_str()).await.unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), name)).await.unwrap();
    for m in migrations {
        pool.execute(*m).await.expect("migration applies");
    }
    pool
}

const M0043: &str = include_str!("../../../migrations/0043_trial_ledger.sql");

fn subject(n: u64) -> TrialSubject {
    TrialSubject {
        config_hash: format!("sha256:cfg{n}"),
        config: serde_json::json!({ "n": n }),
        dataset_id: "slice:sha256:abc".into(),
        split_spec_id: None,
        code_hash: "sha256:code".into(),
        image_digest: "engine@1".into(),
        seed_set: vec![n as i64, 42],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: Some(100),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn trigger_and_rust_hashes_agree_across_a_full_lifecycle() {
    let pool = scratch("ledger_xcheck", &[M0043]).await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("tenant-a", "mason", 0.1 + 0.2).with_experiment("exp-1").on_behalf_of("principal-9");

    let s0 = subject(0);
    let mut t = l.register_async(&Registration::new(&ctx, &s0, f64::MIN_POSITIVE).exploration().candidate_set("sha256:cands")).await.unwrap();
    let id0 = t.trial_id();
    for st in [TrialState::Queued, TrialState::Provision, TrialState::Running, TrialState::Preempted, TrialState::Recovering, TrialState::Running, TrialState::Evaluate, TrialState::Gated] {
        l.transition_async(&mut t, TrialEvent::to(st)).await.unwrap();
    }
    let outcome = OutcomeVector { sharpe_net: Some(1.234_567_890_123), max_dd: Some(-0.2), dd_duration_days: Some(33), model_bytes: Some(1 << 40), ..Default::default() };
    l.settle_async(t, TrialEvent { outcome: Some(outcome), gate_profile: Some("strict_v1".into()), ..TrialEvent::to(TrialState::CompletedPass) }.with_returns("s3://r/0").with_predictions("s3://p/0")).await.unwrap();

    let legacy = DispatchContext::legacy("tenant-a", ledger::ActorKind::Human, "ui");
    let s1 = subject(1);
    let t1 = l.register_async(&Registration::unlogged(&legacy, &s1)).await.unwrap();
    l.settle_async(t1, TrialEvent::failed(TerminalReason::AshaStopped, "rung 2").at_step(40)).await.unwrap();

    let rows = l.trial_rows_async("tenant-a").await.unwrap();
    assert_eq!(verify_trials(&rows).expect("trial chain verifies in Rust"), 2);
    let evs = l.event_rows_async("tenant-a", id0).await.unwrap();
    assert_eq!(evs.len(), 9);
    assert_eq!(verify_events(&rows[0], &evs).expect("event chain verifies in Rust"), 9);
    assert_eq!(evs.last().unwrap().outcome, Some(outcome), "outcome round-trips bit-exact");
    let evs1 = l.event_rows_async("tenant-a", rows[1].trial_id).await.unwrap();
    assert_eq!(verify_events(&rows[1], &evs1).unwrap(), 1);
    assert_eq!(evs1[0].censoring, "right_asha");
    pool.close().await;
}

/// AT-20: the trial table accepts no UPDATE and no DELETE; events are append-only.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn trial_and_events_refuse_mutation() {
    let pool = scratch("ledger_immutable", &[M0043]).await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("t", "m", 0.2).with_experiment("e");
    let s = subject(3);
    let mut t = l.register_async(&Registration::new(&ctx, &s, 0.5)).await.unwrap();
    l.transition_async(&mut t, TrialEvent::to(TrialState::Running)).await.unwrap();
    for stmt in [
        "UPDATE mlops.trial SET propensity = 0.9",
        "UPDATE mlops.trial SET delta_practical = 0.0",
        "DELETE FROM mlops.trial",
        "UPDATE mlops.trial_event SET state = 'completed'",
        "DELETE FROM mlops.trial_event",
        "UPDATE mlops.gate_profile SET thresholds = '{}'",
        "DELETE FROM mlops.gate_profile",
    ] {
        let err = sqlx::query(stmt).execute(&pool).await.expect_err(stmt);
        assert!(err.to_string().contains("append-only"), "{stmt}: {err}");
    }
    pool.close().await;
}

/// The SQL state machine and the Rust one are the same machine.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn sql_and_rust_state_machines_agree() {
    let pool = scratch("ledger_fsm", &[M0043]).await;
    for from in ALL_STATES {
        for to in ALL_STATES {
            let sql: bool = sqlx::query_scalar("SELECT mlops.trial_transition_legal($1, $2)")
                .bind(from.as_str())
                .bind(to.as_str())
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(sql, from.can_transition_to(*to), "{from:?} -> {to:?}");
        }
    }
    pool.close().await;
}

/// The database refuses illegal transitions and unlabelled failures even if a
/// caller bypasses the Rust validation.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn database_enforces_state_machine_and_censoring() {
    let pool = scratch("ledger_db_enforce", &[M0043]).await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("t", "m", 0.2).with_experiment("e");
    let s = subject(4);
    let t = l.register_async(&Registration::new(&ctx, &s, 0.5)).await.unwrap();
    let id = t.trial_id();
    let raw = |state: &str, cens: &str, reason: Option<&str>| {
        let pool = pool.clone();
        let (state, cens, reason) = (state.to_string(), cens.to_string(), reason.map(str::to_string));
        async move {
            sqlx::query("INSERT INTO mlops.trial_event (trial_id, tenant_id, event_seq, state, censoring, terminal_reason, prev_hash, row_hash) VALUES ($1,'t',0,$2,$3,$4,'\\x','\\x')")
                .bind(id).bind(state).bind(cens).bind(reason)
                .execute(&pool).await
        }
    };
    assert!(raw("gated", "none", None).await.is_err(), "registered -> gated is illegal");
    assert!(raw("failed", "none", Some("oom")).await.is_err(), "a failure must be censored");
    assert!(raw("failed", "right_asha", Some("oom")).await.is_err(), "oom is censored 'failed'");
    assert!(raw("running", "none", None).await.is_ok());
    assert!(raw("completed_pass", "none", None).await.is_err(), "cannot pass without the gate");
    // Missing delta_practical outside the legacy marker.
    let err = sqlx::query("INSERT INTO mlops.trial (seq,prev_hash,row_hash,tenant_id,experiment_id,config_hash,config,dataset_id,code_hash,image_digest,seed_set,actor_kind,actor_id,policy_id,policy_version,propensity,exploration_flag,prereg_hash) VALUES (0,'\\x','\\x','t','e','c','{}','d','c','i','{1}','human','m','p',1,0.5,false,'p')")
        .execute(&pool).await.unwrap_err();
    assert!(err.to_string().contains("delta_practical is REQUIRED"));
    pool.close().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a live Postgres"]
async fn decisions_chain_and_require_propensity() {
    let pool = scratch("ledger_decision", &[M0043]).await;
    let l = PgTrialLedger::new(pool.clone());
    let ctx = DispatchContext::human("t", "m", 0.2).with_experiment("e");
    let d = Decision {
        kind: DecisionKind::Select,
        context_hash: "sha256:ctx".into(),
        context_uri: None,
        candidate_set: vec![serde_json::json!({"cfg": 1}), serde_json::json!({"cfg": 2})],
        chosen: serde_json::json!({"cfg": 2}),
        propensity: Some(0.4),
        exploration_flag: false,
        decision_tier: DecisionTier::Rule,
        rationale: Some("acquisition".into()),
    };
    l.log_decision_async(&ctx, &d).await.unwrap();
    l.log_decision_async(&ctx, &d).await.unwrap();
    let seqs: Vec<i64> = sqlx::query_scalar("SELECT seq FROM mlops.decision ORDER BY seq").fetch_all(&pool).await.unwrap();
    assert_eq!(seqs, vec![0, 1]);
    let err = sqlx::query("INSERT INTO mlops.decision (tenant_id,experiment_id,decision_kind,actor_kind,actor_id,context_hash,candidate_set,chosen,policy_id,policy_version,exploration_flag,decision_tier,seq,prev_hash,row_hash) VALUES ('t','e','select','agent','a','c','[1]','1','real',1,false,'rule',0,'\\x','\\x')")
        .execute(&pool).await.unwrap_err();
    assert!(err.to_string().contains("chk_decision_propensity"), "{err}");
    pool.close().await;
}
