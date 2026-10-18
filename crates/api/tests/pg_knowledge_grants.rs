//! The knowledge plane's two new tables (migrations 0183 and 0184) against
//! PostgreSQL, as the role the running platform uses.
//!
//! Every other test connects as the database owner, who can do anything, so a
//! missing or excessive GRANT passes them all and fails live (ADR-P0-13; 0145).
//! This applies the migrations, then acts as `app_role` and `agent_role` - the
//! pattern in `crates/backtest/tests/pg_universe.rs`.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p api --test pg_knowledge_grants -- --ignored
//! ```
//!
//! The scratch database needs `knowledge.insight` (migration 0055) and
//! `mlops.trial`. 0055 needs pgvector; on a server without it, apply the file
//! with `vector(N)` rewritten to `float8[]` first. Nothing here reads the vector
//! columns.

use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

/// 0184 declares `vector`-free columns, so it applies as shipped.
async fn apply(pg: &sqlx::PgPool) {
    sqlx::raw_sql(include_str!("../../../migrations/0183_insight_events.sql"))
        .execute(pg)
        .await
        .expect("0183 applies (and applies twice)");
    sqlx::raw_sql(include_str!(
        "../../../migrations/0184_instrument_fingerprint.sql"
    ))
    .execute(pg)
    .await
    .expect("0184 applies (and applies twice)");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with knowledge.insight; applies 0183 and 0184 itself"]
async fn app_role_appends_insight_events_but_never_edits_or_deletes_one() {
    let pg = pool().await;
    apply(&pg).await;
    apply(&pg).await;

    let tenant = format!("t-{}", Uuid::new_v4().simple());
    let trial = Uuid::new_v4();
    // The insight, as the owner (its own migration is 0055's business).
    let insight: Uuid = sqlx::query_scalar(
        "INSERT INTO knowledge.insight (tenant_id, tier, scope, claim, evidence_trial_ids)
         VALUES ($1, 3, '{}'::jsonb, 'a claim', ARRAY[$2]::uuid[]) RETURNING insight_id",
    )
    .bind(&tenant)
    .bind(trial)
    .fetch_one(&pg)
    .await
    .expect("an insight");

    for role in ["app_role", "agent_role"] {
        let mut tx = pg.begin().await.unwrap();
        sqlx::query(&format!("SET LOCAL ROLE {role}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(&tenant)
            .execute(&mut *tx)
            .await
            .unwrap();
        let ins = sqlx::query(
            "INSERT INTO knowledge.insight_event (tenant_id, insight_id, kind, evidence_trial_ids)
             VALUES ($1, $2, 'contradict', ARRAY[$3]::uuid[])",
        )
        .bind(&tenant)
        .bind(insight)
        .bind(trial)
        .execute(&mut *tx)
        .await;
        assert!(
            ins.is_ok(),
            "{role} cannot append an insight event: {:?}",
            ins.err()
        );
        let read = sqlx::query("SELECT 1 FROM knowledge.insight_event")
            .fetch_all(&mut *tx)
            .await;
        assert!(
            read.is_ok() && read.unwrap().len() == 1,
            "{role} cannot read its tenant's events"
        );
        // Append-only: the counts on the insight are a summary of this log.
        let upd = sqlx::query("UPDATE knowledge.insight_event SET note = 'edited'")
            .execute(&mut *tx)
            .await;
        assert!(upd.is_err(), "{role} can edit an event");
        tx.rollback().await.unwrap();

        let mut tx = pg.begin().await.unwrap();
        sqlx::query(&format!("SET LOCAL ROLE {role}"))
            .execute(&mut *tx)
            .await
            .unwrap();
        let del = sqlx::query("DELETE FROM knowledge.insight_event")
            .execute(&mut *tx)
            .await;
        assert!(del.is_err(), "{role} can delete an event");
        tx.rollback().await.unwrap();
    }

    // Another tenant sees none of it: RLS is forced on the table.
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', 'someone-else', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let seen = sqlx::query("SELECT 1 FROM knowledge.insight_event WHERE insight_id = $1")
        .bind(insight)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert!(seen.is_empty(), "another tenant read this tenant's events");
    tx.rollback().await.unwrap();

    // An event with no evidence is refused by the CHECK, not by a code path.
    let no_evidence = sqlx::query(
        "INSERT INTO knowledge.insight_event (tenant_id, insight_id, kind, evidence_trial_ids)
         VALUES ($1, $2, 'confirm', '{}')",
    )
    .bind(&tenant)
    .bind(insight)
    .execute(&pg)
    .await;
    assert!(
        no_evidence.is_err(),
        "a confirmation with no evidence must be refused"
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL; applies 0184 itself"]
async fn fingerprints_are_written_by_the_platform_and_never_edited() {
    let pg = pool().await;
    apply(&pg).await;
    let instrument: i64 = 9_000_000 + i64::from(Uuid::new_v4().as_u128() as u32 % 1_000_000);

    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    let ins = sqlx::query(
        "INSERT INTO knowledge.instrument_fingerprint
             (instrument_id, venue_id, window_spec, dimension_order, tier1, absent, bars_used, knowledge_time)
         VALUES ($1, 1, '21d@1m', ARRAY['a','b'], ARRAY[0.5, NULL]::real[], '{\"b\":\"not measured\"}', 900, now())",
    )
    .bind(instrument)
    .execute(&mut *tx)
    .await;
    assert!(
        ins.is_ok(),
        "app_role cannot write a fingerprint: {:?}",
        ins.err()
    );
    // NULL is "not measured", and the shape CHECK counts it as an element.
    let bad = sqlx::query(
        "INSERT INTO knowledge.instrument_fingerprint
             (instrument_id, venue_id, window_spec, dimension_order, tier1, bars_used, knowledge_time)
         VALUES ($1, 2, '21d@1m', ARRAY['a','b'], ARRAY[0.5]::real[], 900, now())",
    )
    .bind(instrument)
    .execute(&mut *tx)
    .await;
    assert!(
        bad.is_err(),
        "a tier1 vector shorter than its dimension order must be refused"
    );
    tx.rollback().await.unwrap();

    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE knowledge.instrument_fingerprint SET bars_used = 1")
            .execute(&mut *tx)
            .await
            .is_err(),
        "app_role can edit a fingerprint"
    );
    tx.rollback().await.unwrap();

    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE agent_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query("SELECT 1 FROM knowledge.instrument_fingerprint LIMIT 1")
            .fetch_all(&mut *tx)
            .await
            .is_ok(),
        "agent_role may read fingerprints (market-public)"
    );
    assert!(
        sqlx::query(
            "INSERT INTO knowledge.instrument_fingerprint
                 (instrument_id, venue_id, window_spec, dimension_order, tier1, bars_used, knowledge_time)
             VALUES (1, 1, 'x', ARRAY['a'], ARRAY[1.0]::real[], 1, now())"
        )
        .execute(&mut *tx)
        .await
        .is_err(),
        "agent_role can write a fingerprint"
    );
    tx.rollback().await.unwrap();
}
