//! The session log's constraints, executed rather than read (AGENT-014 §2.1,
//! §5.2, §12.3; Set S task S-11.1).
//!
//! Live Postgres; ignored by default:
//!
//! ```bash
//! cargo test -p invariants --test a_session_log_refuses_what_the_fold_would -- --ignored --test-threads=1
//! ```
//!
//! # Why this exists beside the text checks
//!
//! `a_session_transition_table_has_one_definition` and
//! `exploration_floor_is_one_number` read the migration as text. That catches
//! the failure that actually happens — one copy of a rule edited and not the
//! other — and it runs in every `cargo test` with no services.
//!
//! What it cannot do is prove the constraint *fires*. A CHECK that evaluates to
//! NULL passes; a trigger that is defined but not attached does nothing; a
//! `WHERE` on a partial unique index can be wrong in a way no amount of reading
//! reveals. Those are the three things here, and each of them is one of the
//! acceptance criteria the task was written against, stated adversarially: the
//! test writes the value that must be refused and fails if the database takes
//! it.

use sqlx::{Connection, Executor, PgConnection, PgPool};
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// A database holding `0138` and the four things it references.
///
/// **Not the whole migration chain**, for the reason `semantic_memory_refuses`
/// gives: the chain stops at `0055_knowledge_plane.sql`, which does
/// `CREATE EXTENSION vector`, and a stock Postgres does not have it. A test that
/// insisted on the whole chain would simply never run on a developer machine.
///
/// "Every migration applies to an empty database" is `db_invariants`' claim.
/// This file's claim is narrower: that `0138`'s own constraints fire. So the
/// four objects it references — `users`, `agent_conversations`, `agent_runs` and
/// `mlops.refuse_mutation` — are created as the minimum `0138` needs, and the
/// migration is applied on top. Nothing under test is a constraint on those four,
/// and saying which is which here is the difference between a narrow pass and a
/// claim this run does not support.
async fn migrated() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS session_log_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE session_log_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "session_log_test"))
        .await
        .unwrap();

    pool.execute(
        r#"
        CREATE SCHEMA IF NOT EXISTS mlops;
        CREATE OR REPLACE FUNCTION mlops.refuse_mutation()
        RETURNS TRIGGER LANGUAGE plpgsql AS $fn$
        BEGIN
            RAISE EXCEPTION 'append-only: % on % is refused', TG_OP, TG_TABLE_NAME;
        END;
        $fn$;
        CREATE TABLE users (
            user_id UUID PRIMARY KEY,
            email TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL
        );
        CREATE TABLE agent_conversations (
            conversation_id UUID PRIMARY KEY,
            user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            title TEXT
        );
        CREATE TABLE agent_runs (
            run_id UUID PRIMARY KEY,
            conversation_id UUID REFERENCES agent_conversations(conversation_id),
            turn_index INT NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'queued'
        );
        "#,
    )
    .await
    .expect("the four objects 0138 references");

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crates/<name> sits two below the root")
        .join("migrations/0138_research_sessions.sql");
    let sql = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    pool.execute(sql.as_str())
        .await
        .expect("0138 applies on top of what it references");
    pool
}

/// A user and a conversation for the session to hang from.
async fn scaffold(pool: &PgPool) -> (Uuid, Uuid) {
    let user = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (user_id, email, password_hash)
         VALUES ($1, $2, 'x') ON CONFLICT DO NOTHING",
    )
    .bind(user)
    .bind(format!("{user}@example.invalid"))
    .execute(pool)
    .await
    .expect("a user");
    let convo = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_conversations (conversation_id, user_id, provider, model)
         VALUES ($1, $2, 'anthropic', 'claude')",
    )
    .bind(convo)
    .bind(user)
    .execute(pool)
    .await
    .expect("a conversation");
    (user, convo)
}

async fn continuous_session(pool: &PgPool, share: f64) -> Result<Uuid, sqlx::Error> {
    let (user, convo) = scaffold(pool).await;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO research_sessions
             (session_id, user_id, conversation_id, continuous, charter, tier)
         VALUES ($1, $2, $3, TRUE, $4, 'opus')",
    )
    .bind(id)
    .bind(user)
    .bind(convo)
    .bind(serde_json::json!({ "exploration_share": share, "scope": {},
                              "objective": { "kind": "build_knowledge", "about": [] } }))
    .execute(pool)
    .await?;
    Ok(id)
}

async fn append(
    pool: &PgPool,
    session: Uuid,
    kind: &str,
    state: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO session_events (session_id, kind, state) VALUES ($1, $2, $3)")
        .bind(session)
        .bind(kind)
        .bind(state)
        .execute(pool)
        .await
        .map(|_| ())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_exploration_floor_is_refused_by_the_column_and_not_only_by_rust() {
    let pool = migrated().await;

    // The value AGENT-014 §12.3 names.
    let refused = continuous_session(&pool, 0.05).await;
    assert!(
        refused.is_err(),
        "the column took exploration_share = 0.05. The constructor refuses it, so the platform \
         now refuses it in the one place a human types it and accepts it everywhere else — which \
         is a session that only ever confirms the ranking it already has."
    );

    // At the floor is fine.
    assert!(continuous_session(&pool, 0.10).await.is_ok());

    // A charter with the key *missing* must also be refused: `NULL >= 0.10` is
    // NULL, and a CHECK that evaluates to NULL passes. This is the half a text
    // scan cannot prove.
    let (user, convo) = scaffold(&pool).await;
    let missing = sqlx::query(
        "INSERT INTO research_sessions
             (session_id, user_id, conversation_id, continuous, charter, tier)
         VALUES ($1, $2, $3, TRUE, '{}'::jsonb, 'opus')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(convo)
    .execute(&pool)
    .await;
    assert!(
        missing.is_err(),
        "a charter with no exploration_share was admitted, so the floor can be skipped by \
         omission rather than by being lowered"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_session_is_a_question_or_a_charter_and_the_column_says_so() {
    let pool = migrated().await;
    let (user, convo) = scaffold(&pool).await;

    // Continuous with only a question.
    let both_ways = sqlx::query(
        "INSERT INTO research_sessions
             (session_id, user_id, conversation_id, continuous, question, tier)
         VALUES ($1, $2, $3, TRUE, 'why?', 'opus')",
    )
    .bind(Uuid::new_v4())
    .bind(user)
    .bind(convo)
    .execute(&pool)
    .await;
    assert!(both_ways.is_err(), "a continuous session with no charter");

    // Bounded with both.
    let (user2, convo2) = scaffold(&pool).await;
    let both = sqlx::query(
        "INSERT INTO research_sessions
             (session_id, user_id, conversation_id, continuous, question, charter, tier)
         VALUES ($1, $2, $3, FALSE, 'why?', '{\"exploration_share\": 0.15}'::jsonb, 'opus')",
    )
    .bind(Uuid::new_v4())
    .bind(user2)
    .bind(convo2)
    .execute(&pool)
    .await;
    assert!(
        both.is_err(),
        "a session carrying both a question and a charter"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_trigger_numbers_the_log_and_refuses_an_illegal_transition() {
    let pool = migrated().await;
    let session = continuous_session(&pool, 0.15).await.expect("a session");

    // A log opens in draft and nothing else.
    assert!(
        append(&pool, session, "session_started", Some("running"))
            .await
            .is_err(),
        "a session's log began in `running`, so a session nobody created can be running"
    );
    append(&pool, session, "session_created", Some("draft"))
        .await
        .expect("a log opens in draft");

    // A non-transition event is numbered and says nothing about the state.
    append(&pool, session, "agenda_item_added", None)
        .await
        .expect("a non-transition event");

    append(&pool, session, "session_started", Some("running"))
        .await
        .expect("draft -> running");

    // `running -> draft` is not in the table.
    assert!(
        append(&pool, session, "session_created", Some("draft"))
            .await
            .is_err(),
        "a running session went back to draft"
    );

    append(&pool, session, "session_stopped", Some("stopped"))
        .await
        .expect("running -> stopped");

    // `stopped` is terminal, and that is the property the whole fold rests on:
    // one `psql` session appending past it would make every fold taken
    // afterwards disagree with every fold taken before.
    assert!(
        append(&pool, session, "session_resumed", Some("running"))
            .await
            .is_err(),
        "an event was appended after `stopped`"
    );

    // The writer never chose a position, and the positions are gapless.
    let seqs: Vec<i64> =
        sqlx::query_scalar("SELECT seq FROM session_events WHERE session_id = $1 ORDER BY seq")
            .bind(session)
            .fetch_all(&pool)
            .await
            .expect("the log");
    assert_eq!(seqs, vec![0, 1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn one_live_item_per_key_and_a_finished_question_may_be_asked_again() {
    let pool = migrated().await;
    let session = continuous_session(&pool, 0.15).await.expect("a session");

    let insert = |id: Uuid, status: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query(
                "INSERT INTO agenda_items
                     (item_id, session_id, question, workflow, dedupe_key, status)
                 VALUES ($1, $2, 'does momentum survive costs?', 'investigate',
                         'btc usd|does momentum survive costs', $3)",
            )
            .bind(id)
            .bind(session)
            .bind(status)
            .execute(&pool)
            .await
        }
    };

    let first = Uuid::new_v4();
    insert(first, "queued").await.expect("the first item");
    assert!(
        insert(Uuid::new_v4(), "queued").await.is_err(),
        "two live items share a dedupe key, so layer one of the three-layer refusal is not there"
    );

    // Finish the first. Asking a finished question again is replication, which
    // is something the platform is for.
    sqlx::query("UPDATE agenda_items SET status = 'done' WHERE item_id = $1")
        .bind(first)
        .execute(&pool)
        .await
        .expect("finishing it");
    insert(Uuid::new_v4(), "queued")
        .await
        .expect("a finished question may be asked again");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_log_and_a_digest_cannot_be_rewritten() {
    let pool = migrated().await;
    let session = continuous_session(&pool, 0.15).await.expect("a session");
    append(&pool, session, "session_created", Some("draft"))
        .await
        .expect("a log");

    assert!(
        sqlx::query("UPDATE session_events SET kind = 'session_stopped' WHERE session_id = $1")
            .bind(session)
            .execute(&pool)
            .await
            .is_err(),
        "the log was edited, which makes it a record of the present"
    );
    assert!(
        sqlx::query("DELETE FROM session_events WHERE session_id = $1")
            .bind(session)
            .execute(&pool)
            .await
            .is_err(),
        "the log was deleted from"
    );

    sqlx::query(
        "INSERT INTO session_digests (digest_id, session_id, from_seq, to_seq, body)
         VALUES ($1, $2, 0, 0, '{}'::jsonb)",
    )
    .bind(Uuid::new_v4())
    .bind(session)
    .execute(&pool)
    .await
    .expect("a digest");
    assert!(
        sqlx::query("UPDATE session_digests SET body = '{\"x\":1}'::jsonb WHERE session_id = $1")
            .bind(session)
            .execute(&pool)
            .await
            .is_err(),
        "a digest was rewritten, which changes what the human was told they read"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn an_item_is_retried_once_and_the_column_holds_it() {
    let pool = migrated().await;
    let session = continuous_session(&pool, 0.15).await.expect("a session");
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agenda_items (item_id, session_id, question, workflow, dedupe_key)
         VALUES ($1, $2, 'q', 'investigate', 'k')",
    )
    .bind(id)
    .bind(session)
    .execute(&pool)
    .await
    .expect("an item");

    for n in 1..=2 {
        sqlx::query("UPDATE agenda_items SET attempts = $2 WHERE item_id = $1")
            .bind(id)
            .bind(n)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("attempt {n} refused: {e}"));
    }
    assert!(
        sqlx::query("UPDATE agenda_items SET attempts = 3 WHERE item_id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .is_err(),
        "an item climbed past its retry limit, so §6.6's `expire at 2` is enforced by whoever \
         remembers to count"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_conversation_has_at_most_one_session() {
    let pool = migrated().await;
    let (user, convo) = scaffold(&pool).await;
    for n in 0..2 {
        let r = sqlx::query(
            "INSERT INTO research_sessions
                 (session_id, user_id, conversation_id, continuous, question, tier)
             VALUES ($1, $2, $3, FALSE, 'why?', 'opus')",
        )
        .bind(Uuid::new_v4())
        .bind(user)
        .bind(convo)
        .execute(&pool)
        .await;
        if n == 0 {
            r.expect("the first session");
        } else {
            assert!(
                r.is_err(),
                "a conversation got two sessions, so `the session for this conversation` is a \
                 search with a tie-break rather than a lookup"
            );
        }
    }
}
