//! Live-Postgres tests for the falsification ledger (migration 0059).
//!
//! Every constraint here is also enforced by a sealed Rust type. That is not
//! redundancy: a direct SQL client bypasses the type and does not bypass a
//! CHECK, and the `array_length` hole is the standing proof that a constraint
//! nobody exercised against a real database is a constraint nobody has tested.
//! Reading the DDL would never have found that one, because the SQL said
//! exactly what it meant.
//!
//! ```text
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -p ledger --test pg_falsification -- --ignored --test-threads=1
//! ```

use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    PgPool::connect(&url).await.expect("connect")
}

fn tenant() -> String {
    format!("t_{}", Uuid::new_v4().simple())
}

/// A trial to hang kills off. Registered through raw SQL because this suite is
/// about 0059's constraints, not about the trial ledger's.
async fn a_trial(pool: &PgPool, tenant: &str) -> Uuid {
    let mut tx = ledger::pg::tenant_tx(pool, tenant).await.expect("tx");
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO mlops.trial (
             seq, prev_hash, row_hash, tenant_id, experiment_id, config_hash, config,
             dataset_id, code_hash, image_digest, seed_set, actor_kind, actor_id,
             policy_id, policy_version, propensity, exploration_flag, prereg_hash,
             delta_practical
         ) VALUES (0,'\\x'::bytea,'\\x'::bytea,$1,'e','c','{}'::jsonb,'d','c','i',
                   ARRAY[1]::bigint[],'agent','a','p',1,0.5,false,'p',0.1)
         RETURNING trial_id",
    )
    .bind(tenant)
    .fetch_one(&mut *tx)
    .await
    .expect("trial");
    tx.commit().await.expect("commit");
    id
}

/// `insert` with the given overrides, returning the database's answer.
async fn try_kill(
    pool: &PgPool,
    tenant: &str,
    trial: Uuid,
    class: &str,
    expected: &str,
    observed: &str,
    steps: Vec<String>,
) -> Result<Uuid, sqlx::Error> {
    let mut tx = ledger::pg::tenant_tx(pool, tenant).await.expect("tx");
    let r = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO mlops.falsification (
             tenant_id, report_kind, target_trial_id, defect_class,
             artifact_ref, seed, expected, observed, steps, reported_by, reporter_kind
         ) VALUES ($1,'kill',$2,$3,'run_1',42,$4,$5,$6,'agent_1','agent')
         RETURNING falsification_id",
    )
    .bind(tenant)
    .bind(trial)
    .bind(class)
    .bind(expected)
    .bind(observed)
    .bind(&steps)
    .fetch_one(&mut *tx)
    .await;
    if r.is_ok() {
        tx.commit().await.expect("commit");
    }
    r
}

// ── the constraint that makes a bare complaint unwritable ────────────────────

/// "Sharpe is only 0.4" reduces to `expected == observed`. The database refuses
/// it, not only the Rust constructor.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_report_that_refutes_nothing_is_refused_by_the_database() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;

    let err = try_kill(
        &pool,
        &t,
        trial,
        "invalid_metric",
        "sharpe is 0.4",
        "sharpe is 0.4",
        vec!["look at it".into()],
    )
    .await
    .expect_err("expected == observed must be refused");
    assert!(
        err.to_string()
            .contains("chk_falsification_refutes_something"),
        "wrong constraint fired: {err}"
    );
}

/// Whitespace must not make a claim falsifiable — the CHECK trims both sides.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn whitespace_does_not_make_a_claim_falsifiable() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;

    assert!(try_kill(
        &pool,
        &t,
        trial,
        "lookahead",
        "  same ",
        "same",
        vec!["s".into()]
    )
    .await
    .is_err());
}

/// **The `array_length` lesson, applied.** An empty `steps` array must be
/// refused. `cardinality` returns 0 for one; `array_length` would return NULL
/// and the CHECK would pass.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_report_with_no_steps_is_refused() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;

    let err = try_kill(&pool, &t, trial, "lookahead", "a", "b", vec![])
        .await
        .expect_err("an empty steps array must be refused");
    assert!(
        err.to_string().contains("chk_falsification_has_steps"),
        "wrong constraint fired — if this passed, `cardinality` has been changed back to \
         `array_length` and the CHECK is evaluating to NULL: {err}"
    );
}

/// There is no class for underperformance, and the database says so.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_defect_class_that_names_a_threshold_is_refused() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;

    assert!(
        try_kill(
            &pool,
            &t,
            trial,
            "underperforms",
            "a",
            "b",
            vec!["s".into()]
        )
        .await
        .is_err(),
        "'underperforms' is a result, not a defect, and must not be storable"
    );
}

/// A kill carrying a defect's fields, or the reverse, would score wrong.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_report_may_not_carry_the_other_kinds_fields() {
    let pool = pool().await;
    let t = tenant();
    let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");

    let err = sqlx::query(
        "INSERT INTO mlops.falsification (
             tenant_id, report_kind, component, claimed_guarantee, defect_class,
             artifact_ref, seed, expected, observed, steps, reported_by, reporter_kind
         ) VALUES ($1,'evaluator_defect','sim','no intrabar fills','lookahead',
                   'run_1',1,'a','b',ARRAY['s'],'a','agent')",
    )
    .bind(&t)
    .execute(&mut *tx)
    .await
    .expect_err("a defect must not carry a kill's defect_class");
    assert!(
        err.to_string().contains("chk_falsification_kind_fields"),
        "{err}"
    );
}

// ── immutability of the claim, mutability of the verdict ─────────────────────

/// A reporter must not be able to edit a claim after somebody tried to
/// reproduce it.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_claim_is_immutable_after_it_is_filed() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;
    let id = try_kill(&pool, &t, trial, "lookahead", "a", "b", vec!["s".into()])
        .await
        .expect("filed");

    let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");
    let err =
        sqlx::query("UPDATE mlops.falsification SET observed = $1 WHERE falsification_id = $2")
            .bind("something more convenient")
            .bind(id)
            .execute(&mut *tx)
            .await
            .expect_err("editing the claim must be refused");
    assert!(err.to_string().contains("immutable"), "{err}");
}

/// A refuted report is kept: a pattern of them is the signal that a reporter is
/// guessing.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_report_cannot_be_deleted() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;
    let id = try_kill(&pool, &t, trial, "lookahead", "a", "b", vec!["s".into()])
        .await
        .expect("filed");

    let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");
    let err = sqlx::query("DELETE FROM mlops.falsification WHERE falsification_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect_err("deleting a report must be refused");
    assert!(err.to_string().contains("append-only"), "{err}");
}

/// Verification must remain writable, or nothing can ever be verified.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn the_verdict_is_writable_and_needs_an_author() {
    let pool = pool().await;
    let t = tenant();
    let trial = a_trial(&pool, &t).await;
    let id = try_kill(&pool, &t, trial, "lookahead", "a", "b", vec!["s".into()])
        .await
        .expect("filed");

    // A verdict with no author is refused.
    {
        let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");
        assert!(
            sqlx::query(
                "UPDATE mlops.falsification SET verification='verified' WHERE falsification_id=$1"
            )
            .bind(id)
            .execute(&mut *tx)
            .await
            .is_err(),
            "a verdict without verified_by must be refused"
        );
    }

    let ok = ledger::falsification_store::verify(
        &pool,
        &t,
        id,
        ledger::falsification::Verification::Verified,
        "mason",
    )
    .await
    .expect("verify");
    assert!(ok);
}

// ── scoring ──────────────────────────────────────────────────────────────────

/// The ordering the whole mechanism rests on, read back from the database
/// rather than recomputed: a verified evaluator defect outranks a promotion,
/// and an unverified report of either kind scores zero.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_verified_defect_outranks_a_promotion_and_an_unverified_one_scores_zero() {
    use ledger::falsification::{Score, Verification};

    let pool = pool().await;
    let t = tenant();

    let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");
    let defect: Uuid = sqlx::query_scalar(
        "INSERT INTO mlops.falsification (
             tenant_id, report_kind, component, claimed_guarantee,
             artifact_ref, seed, expected, observed, steps, reported_by, reporter_kind
         ) VALUES ($1,'evaluator_defect','sim.fill_model','fills never cross the bar',
                   'run_1',1,'fills inside the bar','filled at the intrabar low',
                   ARRAY['replay run_1','inspect fill vs bar range'],'agent_1','agent')
         RETURNING falsification_id",
    )
    .bind(&t)
    .fetch_one(&mut *tx)
    .await
    .expect("filed");
    tx.commit().await.expect("commit");

    assert_eq!(
        ledger::falsification_store::score_of(&pool, &t, defect)
            .await
            .expect("score"),
        Some(Score::Zero),
        "an unverified report scores zero, not a discount"
    );

    ledger::falsification_store::verify(&pool, &t, defect, Verification::Verified, "mason")
        .await
        .expect("verify");

    let score = ledger::falsification_store::score_of(&pool, &t, defect)
        .await
        .expect("score")
        .expect("present");
    assert_eq!(score, Score::EvaluatorDefect);
    assert!(
        score.outranks_promotion(),
        "if this ever fails, an agent that finds a simulator bug is better off exploiting it"
    );
}

/// Verification closes the question; only a regression test closes the hole.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn a_verified_defect_stays_on_the_queue_until_a_test_covers_it() {
    use ledger::falsification::Verification;

    let pool = pool().await;
    let t = tenant();

    let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO mlops.falsification (
             tenant_id, report_kind, component, claimed_guarantee,
             artifact_ref, seed, expected, observed, steps, reported_by, reporter_kind
         ) VALUES ($1,'evaluator_defect','migrations','REQUIRED arrays are non-empty',
                   'm_0049',1,'an insight with no evidence is refused',
                   'the database accepted it',ARRAY['insert one'],'agent_1','agent')
         RETURNING falsification_id",
    )
    .bind(&t)
    .fetch_one(&mut *tx)
    .await
    .expect("filed");
    tx.commit().await.expect("commit");

    ledger::falsification_store::verify(&pool, &t, id, Verification::Verified, "mason")
        .await
        .expect("verify");
    assert_eq!(
        ledger::falsification_store::undischarged(&pool, &t)
            .await
            .expect("queue"),
        vec![id]
    );

    ledger::falsification_store::covered_by(
        &pool,
        &t,
        id,
        "no_check_requires_a_non_empty_array_with_array_length",
    )
    .await
    .expect("cover");
    assert!(
        ledger::falsification_store::undischarged(&pool, &t)
            .await
            .expect("queue")
            .is_empty(),
        "a covered defect leaves the queue"
    );

    let mut tx = ledger::pg::tenant_tx(&pool, &t).await.expect("tx");
    let discharged: bool = sqlx::query(
        "SELECT is_discharged FROM mlops.falsification_scored WHERE falsification_id=$1",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .expect("view")
    .get("is_discharged");
    assert!(discharged);
}

// ── tenancy ──────────────────────────────────────────────────────────────────

/// RLS, same shape as every other mlops table.
///
/// Connects as `platform_app` rather than as the database owner: the owner
/// bypasses row-level security, so an isolation test run as the owner passes
/// whatever the policy says. That is the failure mode this test is most likely
/// to be rewritten into, which is why the role is spelled out here.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn one_tenant_cannot_see_another_tenants_reports() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let app_pw =
        std::env::var("PLATFORM_DB_APP_PASSWORD").unwrap_or_else(|_| "platform_app_dev".into());

    let owner = pool().await;
    let (a, b) = (tenant(), tenant());
    let trial = a_trial(&owner, &a).await;
    try_kill(&owner, &a, trial, "lookahead", "x", "y", vec!["s".into()])
        .await
        .expect("filed");

    let app = storage::postgres::connect_app(&owner, &url, &app_pw)
        .await
        .expect("platform_app login");

    // Tenant A sees its own report through the runtime role...
    let mut tx = ledger::pg::tenant_tx(&app, &a).await.expect("tx");
    let mine: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.falsification")
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(mine, 1, "the policy must not hide a tenant from itself");
    drop(tx);

    // ...and tenant B sees nothing.
    let mut tx = ledger::pg::tenant_tx(&app, &b).await.expect("tx");
    let theirs: i64 = sqlx::query_scalar("SELECT count(*) FROM mlops.falsification")
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    assert_eq!(theirs, 0, "tenant {b} can see tenant {a}'s reports");
}

/// The agent may file a report and may not verify one: a reporter that can mark
/// its own report verified scores whatever it likes.
#[tokio::test]
#[ignore = "requires a live Postgres"]
async fn agent_role_holds_insert_but_not_update() {
    let pool = pool().await;
    for (privilege, expected) in [
        ("INSERT", true),
        ("SELECT", true),
        ("UPDATE", false),
        ("DELETE", false),
    ] {
        let has: bool = sqlx::query_scalar(
            "SELECT has_table_privilege('agent_role', 'mlops.falsification', $1)",
        )
        .bind(privilege)
        .fetch_one(&pool)
        .await
        .expect("privilege check");
        assert_eq!(
            has, expected,
            "agent_role {privilege} on mlops.falsification should be {expected}: an agent that              can verify its own report makes the scoring ordering decorative (ADR-P6-06)"
        );
    }
}
