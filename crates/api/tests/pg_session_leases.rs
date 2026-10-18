//! `session_leases` (migration 0160) against live Postgres — AGENT-014 §6.3,
//! §12.1, §12.4; Set S tasks S-11.1 and S-11.5.
//!
//! Ignored by default; run against a scratch database migrated through 0160:
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/scratch \
//!   cargo test -j 2 -p api --test pg_session_leases -- --ignored --test-threads=1
//! ```
//!
//! What is asserted is what the driver relies on and no Rust test can see:
//!
//! * **one child, one lease** — a restart that re-derives the same child and
//!   records its lease again writes nothing, and a second *live* lease on one
//!   agenda item is refused by the schema whatever run it names;
//! * **a lease is committed in full** — its size never changes, it is released
//!   once, and it is never deleted;
//! * **the platform can reach it** — every statement the driver issues runs
//!   here under `SET LOCAL ROLE app_role`, because the suite connects as the
//!   owner and an owner-only test is how 0145 and 0147 came to be needed.
//!
//! Every test runs in a transaction it never commits (the reason is
//! `pg_agent_effect_log.rs`'s header: an append-only table cannot be cleaned up
//! after).

use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/scratch_s11".into())
}

async fn begin() -> (PgPool, Transaction<'static, Postgres>) {
    let pool = PgPool::connect(&url()).await.expect("the database is up");
    let tx = pool.begin().await.expect("begin");
    (pool, tx)
}

async fn probe(conn: &mut PgConnection) {
    sqlx::query("SAVEPOINT probe")
        .execute(conn)
        .await
        .expect("savepoint");
}

async fn undo(conn: &mut PgConnection) {
    sqlx::query("ROLLBACK TO SAVEPOINT probe")
        .execute(conn)
        .await
        .expect("rollback to savepoint");
}

/// A session and one agenda item to lease against, written as the owner.
async fn an_item(conn: &mut PgConnection) -> (Uuid, Uuid) {
    let user = Uuid::new_v4();
    let convo = Uuid::new_v4();
    let session = Uuid::new_v4();
    let item = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("test-fixture-{user}@test.invalid"))
        .execute(&mut *conn)
        .await
        .expect("a user");
    sqlx::query(
        "INSERT INTO agent_conversations (conversation_id, user_id, provider, model)
         VALUES ($1, $2, 'test-fixture', 'test-fixture')",
    )
    .bind(convo)
    .bind(user)
    .execute(&mut *conn)
    .await
    .expect("a conversation");
    sqlx::query(
        "INSERT INTO research_sessions (session_id, conversation_id, user_id, tier, continuous, question)
         VALUES ($1, $2, $3, 'test-fixture', false, 'does the fixture work?')",
    )
    .bind(session)
    .bind(convo)
    .bind(user)
    .execute(&mut *conn)
    .await
    .expect("a session");
    sqlx::query(
        "INSERT INTO agenda_items (item_id, session_id, question, workflow, dedupe_key)
         VALUES ($1, $2, 'does the fixture work?', 'investigate', $3)",
    )
    .bind(item)
    .bind(session)
    .bind(format!("fixture:{item}"))
    .execute(&mut *conn)
    .await
    .expect("an item");
    (session, item)
}

async fn as_app_role(conn: &mut PgConnection) {
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(conn)
        .await
        .expect("the platform's role exists");
}

/// The driver's own INSERT (`SessionWorker::record_lease`), verbatim.
async fn record(
    conn: &mut PgConnection,
    session: Uuid,
    item: Uuid,
    run: Uuid,
    size: f64,
) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "INSERT INTO session_leases
             (session_id, item_id, run_id, size, unit, capped_by, estimated)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT DO NOTHING",
    )
    .bind(session)
    .bind(item)
    .bind(run)
    .bind(size)
    .bind("trials")
    .bind("per_task")
    .bind(false)
    .execute(conn)
    .await
    .map(|r| r.rows_affected())
}

/// The driver's own UPDATE (`SessionWorker::release_lease`), verbatim.
async fn release(conn: &mut PgConnection, session: Uuid, run: Uuid) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "UPDATE session_leases SET released_at = now()
          WHERE session_id = $1 AND run_id = $2 AND released_at IS NULL",
    )
    .bind(session)
    .bind(run)
    .execute(conn)
    .await
    .map(|r| r.rows_affected())
}

/// The driver's own read (`SessionWorker::committed_leases`), summed.
async fn committed(conn: &mut PgConnection, session: Uuid) -> f64 {
    sqlx::query(
        "SELECT COALESCE(SUM(size), 0)::float8 AS s FROM session_leases
          WHERE session_id = $1 AND released_at IS NULL",
    )
    .bind(session)
    .fetch_one(conn)
    .await
    .expect("readable")
    .get::<f64, _>("s")
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0160"]
async fn a_restarted_driver_records_one_lease_for_the_one_child() {
    let (_pool, mut tx) = begin().await;
    let (session, item) = an_item(&mut tx).await;
    as_app_role(&mut tx).await;

    let run = Uuid::new_v4();
    assert_eq!(record(&mut tx, session, item, run, 40.0).await.unwrap(), 1);
    // The restart: `start_child` finds the run already on the derived
    // conversation and the driver records its lease again.
    assert_eq!(
        record(&mut tx, session, item, run, 40.0).await.unwrap(),
        0,
        "one child, one lease"
    );
    // A second live lease on the same item under another run id is the bug the
    // partial unique index exists for — `ON CONFLICT DO NOTHING` swallows it
    // rather than committing the item's budget twice.
    assert_eq!(
        record(&mut tx, session, item, Uuid::new_v4(), 40.0)
            .await
            .unwrap(),
        0
    );
    assert!((committed(&mut tx, session).await - 40.0).abs() < 1e-9);

    // Released, and the item may be leased again for its retry.
    assert_eq!(release(&mut tx, session, run).await.unwrap(), 1);
    assert_eq!(committed(&mut tx, session).await, 0.0);
    let retry = Uuid::new_v4();
    assert_eq!(
        record(&mut tx, session, item, retry, 40.0).await.unwrap(),
        1
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0160"]
async fn three_live_leases_are_summed_and_a_finished_one_is_not() {
    let (_pool, mut tx) = begin().await;
    let mut sessions_items = Vec::new();
    let (session, first) = an_item(&mut tx).await;
    sessions_items.push(first);
    for _ in 0..2 {
        let item = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agenda_items (item_id, session_id, question, workflow, dedupe_key)
             VALUES ($1, $2, 'another', 'investigate', $3)",
        )
        .bind(item)
        .bind(session)
        .bind(format!("fixture:{item}"))
        .execute(&mut *tx)
        .await
        .expect("an item");
        sessions_items.push(item);
    }
    as_app_role(&mut tx).await;
    let runs: Vec<Uuid> = (0..3).map(|_| Uuid::new_v4()).collect();
    for (item, run) in sessions_items.iter().zip(&runs) {
        record(&mut tx, session, *item, *run, 6.0).await.unwrap();
    }
    assert!((committed(&mut tx, session).await - 18.0).abs() < 1e-9);
    release(&mut tx, session, runs[0]).await.unwrap();
    assert!((committed(&mut tx, session).await - 12.0).abs() < 1e-9);
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0160"]
async fn a_lease_is_committed_in_full_and_released_once_and_never_deleted() {
    let (_pool, mut tx) = begin().await;
    let (session, item) = an_item(&mut tx).await;
    as_app_role(&mut tx).await;
    let run = Uuid::new_v4();
    record(&mut tx, session, item, run, 10.0).await.unwrap();

    probe(&mut tx).await;
    let grown = sqlx::query("UPDATE session_leases SET size = 99 WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await;
    assert!(grown.is_err(), "a lease's size is never rewritten");
    undo(&mut tx).await;

    probe(&mut tx).await;
    let deleted = sqlx::query("DELETE FROM session_leases WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await
        .expect_err("the platform role holds no DELETE");
    assert!(
        deleted.to_string().contains("permission denied"),
        "refused by privilege first: {deleted}"
    );
    undo(&mut tx).await;

    assert_eq!(release(&mut tx, session, run).await.unwrap(), 1);
    // The driver's own release is conditional, so a second one is a no-op...
    assert_eq!(release(&mut tx, session, run).await.unwrap(), 0);
    // ...and a writer that went round it is refused by the trigger.
    probe(&mut tx).await;
    let twice = sqlx::query("UPDATE session_leases SET released_at = now() WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await;
    assert!(twice.is_err(), "a lease is released once");
    undo(&mut tx).await;
}

#[tokio::test]
#[ignore = "needs DATABASE_URL migrated through 0160"]
async fn a_lease_that_buys_nothing_or_names_no_unit_is_refused() {
    let (_pool, mut tx) = begin().await;
    let (session, item) = an_item(&mut tx).await;
    as_app_role(&mut tx).await;

    for size in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        probe(&mut tx).await;
        assert!(
            record(&mut tx, session, item, Uuid::new_v4(), size)
                .await
                .is_err(),
            "a lease of {size} is not a lease"
        );
        undo(&mut tx).await;
    }

    probe(&mut tx).await;
    let dollars = sqlx::query(
        "INSERT INTO session_leases (session_id, item_id, run_id, size, unit, capped_by, estimated)
         VALUES ($1, $2, $3, 5, 'dollars', 'per_task', false)",
    )
    .bind(session)
    .bind(item)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await;
    assert!(dollars.is_err(), "the unit is one of 0139's two words");
    undo(&mut tx).await;
}
