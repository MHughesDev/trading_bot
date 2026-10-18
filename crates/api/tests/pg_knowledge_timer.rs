//! The knowledge plane's timer and the insight store's life cycle, against
//! PostgreSQL (OPEN-TASKS C1.3, C1.5; SPEC §5.5-5.6; checklist 3.7, 3.10).
//!
//! `TensorProjection::rebuild` and the insight decay were complete and had no
//! caller. The first time the projection's query was actually run it failed:
//! it read `mlops.trial_state.outcome_vector`, a JSONB column the view has never
//! had (the column is `outcome`, a composite). A test that runs it against a
//! settled trial is what finds that.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p api --test pg_knowledge_timer -- --ignored --test-threads=1
//! ```
//!
//! The scratch database needs migrations through 0183. pgvector is not needed
//! (the embedding column is never read or written here); where the server has
//! none, apply 0055 with `vector(N)` rewritten to `float8[]`.

use api::knowledge_jobs::{
    decayed_score, run_once, InsightError, InsightStore, TensorProjection, HALF_LIFE_DAYS,
    INSIGHT_TOKEN_CAP,
};
use ledger::pg::PgTrialLedger;
use ledger::{DispatchContext, OutcomeVector, Registration, TrialEvent, TrialState, TrialSubject};
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

/// A user row, because a tenant here is a user id.
async fn user(pg: &sqlx::PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("{id}@test.invalid"))
        .execute(pg)
        .await
        .expect("a user");
    id
}

/// A trial settled with a Sharpe, the way the job store settles one.
async fn settled_trial(led: &PgTrialLedger, tenant: &str, sharpe: Option<f64>) -> Uuid {
    let ctx = DispatchContext::human(tenant, "t", 0.1).with_experiment("exp-timer");
    let subject = TrialSubject {
        config_hash: format!("sha256:{}", Uuid::new_v4().simple()),
        config: serde_json::json!({}),
        dataset_id: "slice:x".into(),
        split_spec_id: None,
        code_hash: "sha256:code".into(),
        image_digest: "engine@1".into(),
        seed_set: vec![1],
        non_reproducible: false,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    };
    let mut t = led
        .register_async(&Registration::new(&ctx, &subject, 0.5))
        .await
        .expect("register");
    let id = t.trial_id();
    for st in [
        TrialState::Queued,
        TrialState::Provision,
        TrialState::Running,
        TrialState::Evaluate,
        TrialState::Gated,
    ] {
        led.transition_async(&mut t, TrialEvent::to(st))
            .await
            .unwrap();
    }
    led.settle_async(
        t,
        TrialEvent {
            outcome: Some(OutcomeVector {
                sharpe_net: sharpe,
                ..Default::default()
            }),
            ..TrialEvent::to(TrialState::CompletedPass)
        }
        // A completed trial's return series was persisted, and the CHECK says so.
        .with_returns("s3://r/test"),
    )
    .await
    .unwrap();
    id
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn the_timer_rebuilds_the_tensor_and_is_idempotent() {
    let pg = pool().await;
    let tenant = user(&pg).await.to_string();
    let led = PgTrialLedger::new(pg.clone());
    settled_trial(&led, &tenant, Some(1.25)).await;
    settled_trial(&led, &tenant, Some(-0.4)).await;
    settled_trial(&led, &tenant, None).await; // a trial with no Sharpe: skipped, counted

    let p = TensorProjection::new(pg.clone());
    let first = p
        .rebuild(&tenant)
        .await
        .expect("the projection's query runs");
    assert_eq!(first.facts_written, 2, "{first:?}");
    assert_eq!(first.skipped_without_metric, 1, "{first:?}");
    let second = p.rebuild(&tenant).await.unwrap();
    let (n,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM knowledge.outcome_tensor_fact WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pg)
            .await
            .unwrap();
    assert_eq!(
        n, 2,
        "a rebuild upserts; it does not double the facts ({second:?})"
    );
    let (value,): (f64,) = sqlx::query_as(
        "SELECT max(metric_value) FROM knowledge.outcome_tensor_fact WHERE tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pg)
    .await
    .unwrap();
    assert!((value - 1.25).abs() < 1e-9);

    // The timer's pass reaches the same tenant through the users table.
    let report = run_once(&pg, None).await;
    assert!(report.tenants >= 1);
    assert_eq!(report.errors, 0, "{report:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn an_insight_needs_real_evidence_and_decays_when_contradicted() {
    let pg = pool().await;
    let tenant = user(&pg).await.to_string();
    let led = PgTrialLedger::new(pg.clone());
    let trial = settled_trial(&led, &tenant, Some(1.0)).await;
    let store = InsightStore::new(pg.clone());
    let scope = serde_json::json!({ "regime": "high_vol" });

    // Evidence that is not a trial this tenant ran is a rumour.
    let rumour = store
        .create(
            &tenant,
            3,
            &scope,
            "momentum fails in high vol",
            &[Uuid::new_v4()],
        )
        .await;
    assert!(
        matches!(rumour, Err(InsightError::UnknownEvidence(1))),
        "{rumour:?}"
    );
    let empty = store.create(&tenant, 3, &scope, "no evidence", &[]).await;
    assert!(matches!(empty, Err(InsightError::Invalid(_))), "{empty:?}");

    let a = store
        .create(&tenant, 3, &scope, "momentum fails in high vol", &[trial])
        .await
        .expect("an evidenced claim");
    let b = store
        .create(
            &tenant,
            3,
            &scope,
            "mean reversion holds in low vol",
            &[trial],
        )
        .await
        .unwrap();

    // Contradict one twice; the other is confirmed once.
    let s1 = store
        .contradict(&tenant, a, &[trial], Some("a counter-example"))
        .await
        .unwrap();
    assert!((s1 - 0.5).abs() < 1e-6, "{s1}");
    let s2 = store.contradict(&tenant, a, &[trial], None).await.unwrap();
    assert!((s2 - 0.25).abs() < 1e-6, "{s2}");
    let kept = store.confirm(&tenant, b, &[trial], None).await.unwrap();
    assert!(kept > 0.99, "{kept}");

    // A contradiction with no evidence is refused, like an insight with none.
    assert!(store.contradict(&tenant, a, &[], None).await.is_err());
    // And one for a claim that does not exist is NotFound.
    assert!(matches!(
        store
            .contradict(&tenant, Uuid::new_v4(), &[trial], None)
            .await,
        Err(InsightError::NotFound)
    ));

    // The live claim ranks above the contradicted one.
    let (found, _) = store.search(&tenant, None, 10).await.unwrap();
    assert_eq!(found[0].insight_id, b);
    assert_eq!(found[1].insight_id, a);
    assert_eq!(found[1].contradicted_n, 2);

    // The events are on the log, with their evidence.
    let (events,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM knowledge.insight_event WHERE tenant_id = $1 AND cardinality(evidence_trial_ids) >= 1",
    )
    .bind(&tenant)
    .fetch_one(&pg)
    .await
    .unwrap();
    assert_eq!(events, 3);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn the_decay_sweep_is_idempotent_and_ages_claims_by_their_last_confirmation() {
    let pg = pool().await;
    let tenant = user(&pg).await.to_string();
    let led = PgTrialLedger::new(pg.clone());
    let trial = settled_trial(&led, &tenant, Some(1.0)).await;
    let store = InsightStore::new(pg.clone());
    let id = store
        .create(&tenant, 2, &serde_json::json!({}), "an old claim", &[trial])
        .await
        .unwrap();

    // Age it one half-life without touching the score.
    sqlx::query(
        "UPDATE knowledge.insight SET created_at = now() - make_interval(days => $2)
          WHERE insight_id = $1",
    )
    .bind(id)
    .bind(HALF_LIFE_DAYS as i32)
    .execute(&pg)
    .await
    .unwrap();

    let changed = store.decay_sweep(&tenant).await.unwrap();
    assert_eq!(changed, 1);
    let (score,): (f32,) =
        sqlx::query_as("SELECT decay_score FROM knowledge.insight WHERE insight_id = $1")
            .bind(id)
            .fetch_one(&pg)
            .await
            .unwrap();
    assert!(
        (score - decayed_score(HALF_LIFE_DAYS, 0)).abs() < 1e-3,
        "{score}"
    );
    assert_eq!(
        store.decay_sweep(&tenant).await.unwrap(),
        0,
        "a second sweep changes nothing: the score is a function of age, not of how often the timer ran"
    );

    // A confirmation resets the clock.
    store.confirm(&tenant, id, &[trial], None).await.unwrap();
    let (score,): (f32,) =
        sqlx::query_as("SELECT decay_score FROM knowledge.insight WHERE insight_id = $1")
            .bind(id)
            .fetch_one(&pg)
            .await
            .unwrap();
    assert!(score > 0.99, "{score}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0183"]
async fn retrieval_never_exceeds_the_token_budget_however_much_is_asked_for() {
    let pg = pool().await;
    let tenant = user(&pg).await.to_string();
    let led = PgTrialLedger::new(pg.clone());
    let trial = settled_trial(&led, &tenant, Some(1.0)).await;
    let store = InsightStore::new(pg.clone());
    // Six claims of ~1 000 tokens each: five would not fit in 4 000.
    for i in 0..6 {
        let claim = format!("{i}{}", "x".repeat(3_999));
        store
            .create(&tenant, 1, &serde_json::json!({ "i": i }), &claim, &[trial])
            .await
            .unwrap();
    }
    let (found, used) = store.search(&tenant, None, 500).await.unwrap();
    assert!(used <= INSIGHT_TOKEN_CAP, "{used}");
    assert_eq!(found.len(), 4);
}
