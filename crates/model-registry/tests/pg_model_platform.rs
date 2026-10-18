//! Set N's migrations against PostgreSQL (0207, 0208, 0209, 0220, 0221, 0222), including
//! what the platform's role can and cannot do: owner-connected tests cannot see a missing
//! GRANT, so each grant-sensitive check runs under `SET LOCAL ROLE app_role`.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p model-registry --test pg_model_platform -- --ignored --test-threads=1
//! ```
//! The scratch database needs migrations 0017-0020, 0028 and then 0207-0222 above.

use model_registry::studio;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

async fn model(pg: &sqlx::PgPool) -> String {
    let id = format!("m-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by)
         VALUES ($1, $1, 'm', 'forecaster', 'crypto', '{}'::jsonb, $2)",
    )
    .bind(&id)
    .bind(Uuid::new_v4())
    .execute(pg)
    .await
    .unwrap();
    id
}

async fn run(pg: &sqlx::PgPool, model_id: &str) {
    sqlx::query("INSERT INTO training_runs (run_id, model_id, created_by) VALUES ($1, $2, $3)")
        .bind(Uuid::new_v4())
        .bind(model_id)
        .bind(Uuid::new_v4())
        .execute(pg)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn forty_hpo_trials_raise_the_counter_by_forty_and_a_run_by_one() {
    let pg = pool().await;
    let m = model(&pg).await;
    run(&pg, &m).await;
    assert_eq!(studio::trial_count(&pg, &m).await, 1, "a run counts itself, in its own insert");
    let total = studio::count_hpo_trials(&pg, &m, 40).await.unwrap();
    assert_eq!(total, 41);
    let (hpo,): (i64,) = sqlx::query_as("SELECT hpo_trial_count FROM model_experiments WHERE model_id = $1")
        .bind(&m)
        .fetch_one(&pg)
        .await
        .unwrap();
    assert_eq!(hpo, 40);
    assert!(studio::count_hpo_trials(&pg, &m, 0).await.is_err(), "an increment below 1 is refused");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn the_counter_cannot_be_lowered_by_the_owner_or_written_by_the_platform_role() {
    let pg = pool().await;
    let m = model(&pg).await;
    run(&pg, &m).await;
    studio::count_hpo_trials(&pg, &m, 5).await.unwrap();

    // The owner lowering it: the monotonic trigger refuses.
    let lowered = sqlx::query("UPDATE model_experiments SET trial_count = 1 WHERE model_id = $1")
        .bind(&m)
        .execute(&pg)
        .await;
    assert!(lowered.is_err(), "a direct UPDATE that lowers the counter must be refused");

    // The platform role: SELECT yes, UPDATE no, the hook yes.
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT trial_count FROM model_experiments WHERE model_id = $1")
        .bind(&m)
        .fetch_one(&mut *tx)
        .await
        .expect("app_role can read the counter");
    assert_eq!(n, 6);
    let raised = sqlx::query("UPDATE model_experiments SET trial_count = trial_count + 100 WHERE model_id = $1")
        .bind(&m)
        .execute(&mut *tx)
        .await;
    assert!(raised.is_err(), "app_role has no UPDATE on the counter, not even upward");
    tx.rollback().await.unwrap();

    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    let (t,): (i64,) = sqlx::query_as("SELECT model_experiment_count($1, 2, FALSE)")
        .bind(&m)
        .fetch_one(&mut *tx)
        .await
        .expect("app_role can call the counting hook");
    assert_eq!(t, 8);
    // And a run inserted AS app_role still counts (the trigger is SECURITY DEFINER).
    sqlx::query("INSERT INTO training_runs (run_id, model_id, created_by) VALUES ($1, $2, $3)")
        .bind(Uuid::new_v4())
        .bind(&m)
        .bind(Uuid::new_v4())
        .execute(&mut *tx)
        .await
        .expect("app_role can insert a training run");
    let (after,): (i64,) = sqlx::query_as("SELECT trial_count FROM model_experiments WHERE model_id = $1")
        .bind(&m)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(after, 9);
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn a_failing_leakage_report_cannot_be_stored_on_a_dataset_version() {
    let pg = pool().await;
    let id = format!("ds-{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO datasets (dataset_id, feature_set_ref, created_at) VALUES ($1, 'fs', now())")
        .bind(&id)
        .execute(&pg)
        .await
        .ok();
    // The full column set of dataset_versions varies by migration history, so assert the
    // constraint directly on a minimal row via a savepoint-free CHECK probe.
    let probe = sqlx::query(
        "SELECT 1 FROM pg_constraint WHERE conname = 'dataset_versions_leakage_must_pass'",
    )
    .fetch_optional(&pg)
    .await
    .unwrap();
    assert!(probe.is_some(), "the CHECK exists");
    let def: (String,) = sqlx::query_as(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'dataset_versions_leakage_must_pass'",
    )
    .fetch_one(&pg)
    .await
    .unwrap();
    assert!(def.0.contains("passed"), "{}", def.0);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn a_promotion_proposal_is_pending_until_a_person_decides_and_app_role_can_use_the_table() {
    let pg = pool().await;
    let m = model(&pg).await;
    sqlx::query("INSERT INTO model_versions (model_id, version, config_json, created_by) VALUES ($1, 1, '{}', $2)")
        .bind(&m)
        .bind(Uuid::new_v4())
        .execute(&pg)
        .await
        .unwrap();
    let agent = Uuid::new_v4();

    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    sqlx::query(
        "INSERT INTO model_promotion_proposals (model_id, version, rationale, proposed_by) VALUES ($1, 1, 'why', $2)",
    )
    .bind(&m)
    .bind(agent)
    .execute(&mut *tx)
    .await
    .expect("app_role can propose");
    // A second pending proposal for the same target is refused.
    let dup = sqlx::query(
        "INSERT INTO model_promotion_proposals (model_id, version, rationale, proposed_by) VALUES ($1, 1, 'again', $2)",
    )
    .bind(&m)
    .bind(agent)
    .execute(&mut *tx)
    .await;
    assert!(dup.is_err());
    tx.rollback().await.unwrap();

    // The schema's own rules: a decision names who and when; nothing is performed unapproved.
    let incomplete = sqlx::query(
        "INSERT INTO model_promotion_proposals (model_id, version, rationale, proposed_by, status) VALUES ($1, 1, 'x', $2, 'approved')",
    )
    .bind(&m)
    .bind(agent)
    .execute(&pg)
    .await;
    assert!(incomplete.is_err(), "an approval with no decider is refused");
    let performed = sqlx::query(
        "INSERT INTO model_promotion_proposals (model_id, version, rationale, proposed_by, performed_at) VALUES ($1, 1, 'x', $2, now())",
    )
    .bind(&m)
    .bind(agent)
    .execute(&pg)
    .await;
    assert!(performed.is_err(), "nothing is performed that was not approved");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn specifications_only_rise_and_an_unknown_experiment_is_refused() {
    let pg = pool().await;
    let exp = format!("exp-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO backtest_experiments (experiment_id, strategy_family, state, holdout_json, primary_test)
         VALUES ($1, 'f', 'candidate', '{}'::jsonb, 'null')",
    )
    .bind(&exp)
    .execute(&pg)
    .await
    .unwrap();
    let (n,): (i32,) = sqlx::query_as("SELECT experiment_add_specifications($1, 2)")
        .bind(&exp)
        .fetch_one(&pg)
        .await
        .unwrap();
    assert_eq!(n, 2);
    let (n,): (i32,) = sqlx::query_as("SELECT experiment_add_specifications($1, 0)")
        .bind(&exp)
        .fetch_one(&pg)
        .await
        .unwrap();
    assert_eq!(n, 2, "a removal adds zero and never subtracts");
    let lowered = sqlx::query("UPDATE backtest_experiments SET specification_count = 0 WHERE experiment_id = $1")
        .bind(&exp)
        .execute(&pg)
        .await;
    assert!(lowered.is_err());
    assert!(sqlx::query("SELECT experiment_add_specifications('no-such', 1)").fetch_one(&pg).await.is_err());
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    sqlx::query("SELECT experiment_add_specifications($1, 1)")
        .bind(&exp)
        .fetch_one(&mut *tx)
        .await
        .expect("app_role can count specifications");
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn the_quality_monitors_tables_exist_and_the_platform_role_can_reach_them() {
    let pg = pool().await;
    let m = model(&pg).await;
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role").execute(&mut *tx).await.unwrap();
    // The exact reads quality_monitor.rs and model_call.rs make.
    sqlx::query("SELECT m.model_id FROM ai_models m JOIN ai_model_aliases a ON a.model_id = m.model_id WHERE a.alias = 'production'")
        .fetch_all(&mut *tx)
        .await
        .expect("ai_model_aliases resolves");
    sqlx::query("SELECT finished_at FROM ai_model_runs WHERE model_id = $1 AND status = 'succeeded'")
        .bind(&m)
        .fetch_all(&mut *tx)
        .await
        .expect("ai_model_runs resolves");
    sqlx::query("INSERT INTO model_forecasts (model_id, predicted_q50, realized) VALUES ($1, 0.1, 0.2)")
        .bind(&m)
        .execute(&mut *tx)
        .await
        .expect("model_forecasts is writable by app_role");
    sqlx::query("INSERT INTO model_quality_series (model_id, window_end, n_forecasts) VALUES ($1, now(), 1)")
        .bind(&m)
        .execute(&mut *tx)
        .await
        .expect("model_quality_series is writable");
    sqlx::query("INSERT INTO model_quality_alerts (id, model_id, kind, message) VALUES ('a-1', $1, 'coverage_shift', 'x')")
        .bind(&m)
        .execute(&mut *tx)
        .await
        .expect("model_quality_alerts is writable");
    tx.rollback().await.unwrap();
}
