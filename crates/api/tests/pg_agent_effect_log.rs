//! The effect log's contract, against live Postgres (AGENT-010 §5; Set S S-2.5).
//!
//! Ignored by default:
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p api --test pg_agent_effect_log -- --ignored --test-threads=1
//! ```
//!
//! What is asserted here is the thing `agent_messages` cannot do. A transcript can
//! tell you a tool ran; it cannot tell you whether the loop had *decided* to run it
//! before the process died, and that is the only question resume actually asks.
//!
//! ## Why every test here runs inside a transaction it never commits
//!
//! `DATABASE_URL` is the platform's *own* development database. An earlier version
//! of this file wrote through the pool: `a_run` minted a fresh `users` row and a
//! fresh `agent_runs` row per call and cleaned up neither, which left 23 of the 55
//! rows in `agent_runs` and all 25 rows in `agent_effect_log` as fixtures. A track
//! reporting "28 runs recorded no token count" was mostly counting them.
//!
//! A `DELETE` teardown cannot fix it. `agent_effect_log` carries
//! `trg_agent_effect_log_immutable` — the property
//! [`the_effect_log_refuses_to_be_edited`] exists to assert — and
//! `agent_effect_log.run_id` is `ON DELETE CASCADE` from `agent_runs`, so deleting
//! a run *cascades into the refusal*:
//!
//! ```text
//! DELETE FROM agent_runs WHERE goal = '…';
//! ERROR: agent_effect_log is append-only: DELETE refused
//! ```
//!
//! A run that has logged a single event can never be removed, and nor can the user
//! it hangs off. There is no teardown to write. The only way to write to an
//! append-only table and leave nothing behind is to not commit, so [`begin`] hands
//! each test a [`Transaction`] and no test commits one. Dropping it rolls back —
//! on the unwind path, the early-return path, and the killed-process path alike.
//! [`the_suite_gives_the_database_back_unchanged`] proves that against a real
//! panic rather than asserting it in a comment.

use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

// ── markers ──────────────────────────────────────────────────────────────────

/// Stamped on every `agent_runs` row this file writes.
///
/// Rollback means nothing should survive to carry it; this is the second line,
/// for the day somebody reintroduces a pool write. The value is chosen so that
/// a row carrying it cannot be mistaken for a real run and cannot be produced
/// by the platform: `crates/api/src/agent/manager.rs` binds the user's own
/// prompt text into `goal`, and while a person could in principle type this
/// string, `provider` and `model` below come from a configured LLM profile and
/// `test-fixture` is not one.
const TEST_GOAL: &str = "test-fixture:api/pg_agent_effect_log";

/// `provider` and `model`. The previous value was `'test'`, which reads like a
/// smoke run somebody meant to keep.
const TEST_PROVIDER: &str = "test-fixture";

/// `harness_version` on every effect-log row, and the version handed to
/// [`decide`] so the two still match.
///
/// Production binds `HARNESS_VERSION`, which is `env!("CARGO_PKG_VERSION")` for
/// the `api` crate — so the old literal `'0.1.0'` was, at the time of writing,
/// byte-identical to what the real harness writes. `-test-fixture` is not a
/// version this crate can ever be published at.
const TEST_HARNESS_VERSION: &str = "0.0.0-test-fixture";

/// `.invalid` is reserved by RFC 2606 and can never resolve, so a row carrying
/// one of these addresses is never a person.
fn test_email(user_id: Uuid) -> String {
    format!("test-fixture-{user_id}@test.invalid")
}

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

async fn pool() -> PgPool {
    PgPool::connect(&url())
        .await
        .expect("the dev database is up")
}

/// A pool and a transaction that must never be committed.
async fn begin() -> (PgPool, Transaction<'static, Postgres>) {
    let pool = pool().await;
    let tx = pool.begin().await.expect("begin");
    (pool, tx)
}

/// Postgres aborts the *whole* transaction on a failed statement, and most of
/// this file asserts refusals. Without a savepoint the second refusal in a test
/// reports `current transaction is aborted` rather than the constraint it meant
/// to check — an `is_err()` that passes while checking nothing.
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

/// A run to hang events off. `agent_effect_log.run_id` is a foreign key, which is the
/// point — an effect log with no run is not evidence about anything.
async fn a_run(conn: &mut PgConnection) -> Uuid {
    // The user is created too rather than borrowed from whatever the dev database
    // happens to hold: a test that depends on a row it did not write passes or
    // fails on the state of somebody else's machine.
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user_id)
        .bind(test_email(user_id))
        .execute(&mut *conn)
        .await
        .expect("a user row");
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_runs (run_id, user_id, goal, status, provider, model) \
         VALUES ($1, $2, $3, 'running', $4, $4)",
    )
    .bind(run_id)
    .bind(user_id)
    .bind(TEST_GOAL)
    .bind(TEST_PROVIDER)
    .execute(&mut *conn)
    .await
    .expect("a run row");
    run_id
}

async fn append(
    conn: &mut PgConnection,
    run: Uuid,
    seq: i32,
    direction: &str,
    kind: &str,
    payload: &str,
) {
    sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, $2, $3, $4, $5::jsonb, $6)",
    )
    .bind(run)
    .bind(seq)
    .bind(direction)
    .bind(kind)
    .bind(payload)
    .bind(TEST_HARNESS_VERSION)
    .execute(&mut *conn)
    .await
    .expect("the event is appended");
}

/// The ordering that makes the log worth keeping: the effect is recorded before the
/// input that answers it.
///
/// A log where the input came first would say a tool returned before anything
/// decided to call it, and a resume reading it could not tell an interrupted write
/// from a completed one.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn an_effect_is_recorded_before_the_input_that_answers_it() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;

    append(
        &mut tx,
        run,
        1,
        "effect",
        "ExecuteTool",
        r#"{"step":1,"name":"create_experiment","arguments":{"slug":"ema_v1"},"idempotent":false}"#,
    )
    .await;
    append(
        &mut tx,
        run,
        2,
        "input",
        "ToolResult",
        r#"{"output":"{\"experiment_id\":\"exp_1\"}","source":"platform"}"#,
    )
    .await;

    let rows = sqlx::query(
        "SELECT seq, direction, kind FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run)
    .fetch_all(&mut *tx)
    .await
    .expect("the log reads back in order");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("direction"), "effect");
    assert_eq!(rows[0].get::<String, _>("kind"), "ExecuteTool");
    assert_eq!(rows[1].get::<String, _>("direction"), "input");
    assert!(
        rows[0].get::<i32, _>("seq") < rows[1].get::<i32, _>("seq"),
        "the decision is recorded before the answer, or a crash between them is invisible"
    );
}

/// The interrupted case, which is the whole reason the log exists: an effect with no
/// answering input.
///
/// A resume finds this by looking for a trailing effect row, and S-2.6 decides from
/// `idempotent` whether re-running it is safe. Neither question is answerable from a
/// transcript.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn an_interrupted_write_is_visible_as_a_trailing_effect() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;

    append(
        &mut tx,
        run,
        1,
        "effect",
        "CallModel",
        r#"{"step":1,"decode":"select_tool"}"#,
    )
    .await;
    append(
        &mut tx,
        run,
        2,
        "input",
        "ModelReply",
        r#"{"raw":"{\"name\":\"create_experiment\"}"}"#,
    )
    .await;
    // Decided, and then the process died.
    append(
        &mut tx,
        run,
        3,
        "effect",
        "ExecuteTool",
        r#"{"step":1,"name":"create_experiment","arguments":{"slug":"ema_v1"},"idempotent":false}"#,
    )
    .await;

    let last = sqlx::query(
        "SELECT direction, kind, payload_json FROM agent_effect_log \
         WHERE run_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(run)
    .fetch_one(&mut *tx)
    .await
    .expect("a last row");

    assert_eq!(last.get::<String, _>("direction"), "effect");
    let payload: serde_json::Value = last.get("payload_json");
    assert_eq!(payload["name"], "create_experiment");
    assert_eq!(
        payload["idempotent"], false,
        "a write, so a resume must NOT re-run it — which it can only know from here"
    );
    assert!(
        payload["arguments"].is_object(),
        "the arguments in full: a hash cannot answer `was this the same call`"
    );
}

/// Append-only, like every other evidence table here. A log somebody edited is a log
/// nobody can rely on, and every later claim about that run becomes unfalsifiable.
///
/// This is also the constraint that makes a `DELETE` teardown impossible for this
/// suite — see the module docs.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn the_effect_log_refuses_to_be_edited() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;
    append(&mut tx, run, 1, "effect", "CallModel", r#"{"step":1}"#).await;

    probe(&mut tx).await;
    let updated = sqlx::query("UPDATE agent_effect_log SET kind = 'Tampered' WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await;
    assert!(updated.is_err(), "an effect log row must not be updatable");
    undo(&mut tx).await;

    probe(&mut tx).await;
    let deleted = sqlx::query("DELETE FROM agent_effect_log WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await;
    assert!(deleted.is_err(), "nor deletable");
    undo(&mut tx).await;
}

/// The cascade that closes the last escape route: deleting the *run* does not get
/// around the log's immutability, it trips it.
///
/// This is not decoration. `agent_effect_log_run_id_fkey` is `ON DELETE CASCADE`,
/// which reads like a tidy-up path and is the opposite — it means a run that logged
/// one event is permanently unremovable, and so is the user it belongs to. Anything
/// that writes runs to this database and expects to clean up afterwards is wrong,
/// and this is where that is recorded.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn deleting_a_run_cannot_launder_its_effect_log_away() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;
    append(&mut tx, run, 1, "effect", "CallModel", r#"{"step":1}"#).await;

    probe(&mut tx).await;
    let err = sqlx::query("DELETE FROM agent_runs WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await
        .expect_err("the cascade reaches an append-only table and is refused");
    assert!(
        err.to_string().contains("agent_effect_log"),
        "the refusal must name the log, so the next reader knows why the run is \
         stuck rather than assuming a permissions problem: {err}"
    );
    undo(&mut tx).await;
}

/// Two rows cannot claim the same position. The order IS the content: a log whose
/// rows cannot be put back in sequence is a set of facts about a run, not the run.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn two_events_cannot_share_a_sequence_number() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;
    append(&mut tx, run, 1, "effect", "CallModel", r#"{"step":1}"#).await;

    probe(&mut tx).await;
    let clash = sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, 1, 'input', 'ModelReply', '{}'::jsonb, $2)",
    )
    .bind(run)
    .bind(TEST_HARNESS_VERSION)
    .execute(&mut *tx)
    .await;
    assert!(clash.is_err(), "seq is part of the key, not a hint");
    undo(&mut tx).await;
}

/// `direction` is closed, so a writer cannot invent a third kind of thing and leave
/// a reader to guess whether it was a decision or an answer.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn a_row_that_is_neither_a_decision_nor_an_answer_is_refused() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;

    probe(&mut tx).await;
    let bad = sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, 1, 'sideways', 'CallModel', '{}'::jsonb, $2)",
    )
    .bind(run)
    .bind(TEST_HARNESS_VERSION)
    .execute(&mut *tx)
    .await;
    assert!(
        bad.is_err(),
        "direction is effect | input | state and nothing else"
    );
    undo(&mut tx).await;
}

/// A row with no harness version cannot be replayed, because nothing says what would
/// be replaying it. Refused rather than defaulted.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn an_event_with_no_harness_version_is_refused() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;

    probe(&mut tx).await;
    let bad = sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, 1, 'effect', 'CallModel', '{}'::jsonb, '')",
    )
    .bind(run)
    .execute(&mut *tx)
    .await;
    assert!(bad.is_err(), "an empty harness version is not a version");
    undo(&mut tx).await;
}

/// The resume decision, made from rows actually read back out of Postgres.
///
/// The unit tests in `agent::resume` cover the shapes. This covers the seam: that
/// what `decide` is handed matches what the writer put in, through JSONB and back.
/// A `payload_json` that lost `idempotent` on the round trip would make every
/// interrupted write look retryable, and the unit tests could not see it.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn the_resume_decision_reads_an_interrupted_write_out_of_the_database() {
    use api::agent::resume::{decide, LogRow, Resume};

    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;
    append(
        &mut tx,
        run,
        1,
        "input",
        "Admission",
        r#"{"admission":"Admit"}"#,
    )
    .await;
    append(
        &mut tx,
        run,
        2,
        "effect",
        "ExecuteTool",
        r#"{"step":1,"name":"create_experiment","arguments":{"slug":"ema_v1"},"idempotent":false}"#,
    )
    .await;

    let rows: Vec<(i32, String, String, serde_json::Value, String)> = sqlx::query_as(
        "SELECT seq, direction, kind, payload_json, harness_version \
         FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run)
    .fetch_all(&mut *tx)
    .await
    .expect("the log reads back");

    let rows: Vec<LogRow> = rows
        .into_iter()
        .map(|(seq, direction, kind, payload, harness_version)| LogRow {
            seq,
            direction,
            kind,
            payload,
            harness_version,
        })
        .collect();

    // The version handed in is the one the rows carry. `decide` refuses to resume a
    // log written by a different harness, so this asserts the round trip and not the
    // version-mismatch branch — which has its own unit test.
    let Resume::Replay { inputs, pending } = decide(&rows, TEST_HARNESS_VERSION) else {
        panic!("a run with a trailing effect resumes");
    };
    assert_eq!(inputs.len(), 1);
    let p = pending.expect("the interrupted write is reported");
    assert!(
        !p.idempotent,
        "`idempotent` survived the JSONB round trip; if it had not, every \
         interrupted write would look retryable"
    );
    assert!(p.interrupted_error().contains("create_experiment"));
}

/// The ten telemetry columns exist and hold what they claim (S-2.8).
///
/// `prompt_tokens` and `tokens_in` are the pair worth asserting apart. With caching
/// on they differ by the hit rate, and a reader who took `tokens_in` for the prompt
/// size would believe a 40,000-token prompt was a thousand — which is how a context
/// budget comes to think it has headroom it does not have.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn a_finished_run_carries_its_telemetry() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;

    sqlx::query(
        "UPDATE agent_runs SET \
             harness_version = $2, profile_name = 'qwen2.5:7b-instruct', \
             adapter = 'ollama', transport = 'select_fill', \
             tokens_in = 1000, prompt_tokens = 40000, est_prompt_tokens = 42000, \
             cache_read_tokens = 39000, trials_spent = 4, n_eff_after = 2.5, \
             outcome = 'finished', status = 'completed' \
         WHERE run_id = $1",
    )
    .bind(run)
    .bind(TEST_HARNESS_VERSION)
    .execute(&mut *tx)
    .await
    .expect("the telemetry writes");

    let r = sqlx::query(
        "SELECT harness_version, profile_name, adapter, transport, tokens_in, \
                prompt_tokens, est_prompt_tokens, cache_read_tokens, trials_spent, \
                n_eff_after, outcome \
         FROM agent_runs WHERE run_id = $1",
    )
    .bind(run)
    .fetch_one(&mut *tx)
    .await
    .expect("it reads back");

    assert_eq!(r.get::<String, _>("harness_version"), TEST_HARNESS_VERSION);
    assert_eq!(r.get::<String, _>("transport"), "select_fill");
    assert_eq!(r.get::<String, _>("outcome"), "finished");

    let billed: i64 = r.get("tokens_in");
    let prompt: i64 = r.get("prompt_tokens");
    let cached: i64 = r.get("cache_read_tokens");
    assert_eq!(billed, 1_000, "what it cost");
    assert_eq!(prompt, 40_000, "how big the prompt was");
    assert!(
        prompt > billed,
        "these are different numbers, and a reader who conflates them believes a \
         40,000-token prompt was a thousand"
    );
    assert_eq!(cached, 39_000);

    // The honesty pair: four trials that moved N_eff by less than four is the same
    // idea searched more than once, and neither number says that alone.
    assert_eq!(r.get::<i32, _>("trials_spent"), 4);
    assert!((r.get::<f64, _>("n_eff_after") - 2.5).abs() < 1e-9);
}

/// A crashed run is still a run: the columns are nullable, and a null is not a zero.
///
/// NOT NULL here would mean either refusing to record the crash or inventing a
/// value for it, and an invented `trials_spent = 0` is worse than a null because a
/// null cannot be averaged by accident.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn a_run_that_never_finished_leaves_its_telemetry_null() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;
    let r = sqlx::query("SELECT trials_spent, outcome FROM agent_runs WHERE run_id = $1")
        .bind(run)
        .fetch_one(&mut *tx)
        .await
        .expect("it reads back");
    assert!(r.get::<Option<i32>, _>("trials_spent").is_none());
    assert!(r.get::<Option<String>, _>("outcome").is_none());
}

/// A negative token count is not a count. Refused rather than stored.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn a_negative_token_count_is_refused() {
    let (_pool, mut tx) = begin().await;
    let run = a_run(&mut tx).await;

    probe(&mut tx).await;
    let bad = sqlx::query("UPDATE agent_runs SET prompt_tokens = -1 WHERE run_id = $1")
        .bind(run)
        .execute(&mut *tx)
        .await;
    assert!(bad.is_err(), "a negative prompt size is not a prompt size");
    undo(&mut tx).await;
}

// ── the suite's own footprint ────────────────────────────────────────────────

/// This suite writes to the platform's development database and must give it back
/// exactly as it found it — *including* when a test panics partway through.
///
/// The panic is real and deliberate: the work runs in a spawned task that unwinds
/// after the user, the run and an effect-log row are all written, so what is
/// asserted below is the unwind path and not the happy one. Cleanup that only runs
/// on the success path is exactly what produced the 23 stranded runs this file is
/// named for, and a version of this check that let the body finish normally would
/// pass against that bug.
///
/// A `DELETE` teardown could not stand in for this: once the log row exists, the
/// run and its user are unremovable by anything short of disabling a trigger. The
/// rollback is not the convenient option, it is the only one.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn the_suite_gives_the_database_back_unchanged() {
    async fn counts(pool: &PgPool) -> (i64, i64, i64) {
        let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(pool)
            .await
            .expect("count users");
        let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_runs")
            .fetch_one(pool)
            .await
            .expect("count runs");
        let events: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_effect_log")
            .fetch_one(pool)
            .await
            .expect("count events");
        (users, runs, events)
    }

    let before_pool = pool().await;
    let before = counts(&before_pool).await;

    let spawned = before_pool.clone();
    let panicked = tokio::spawn(async move {
        let mut tx = spawned.begin().await.expect("begin");
        let run = a_run(&mut tx).await;
        append(&mut tx, run, 1, "effect", "CallModel", r#"{"step":1}"#).await;

        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_effect_log WHERE run_id = $1")
            .bind(run)
            .fetch_one(&mut *tx)
            .await
            .expect("count");
        assert_eq!(n, 1, "the fixture really was written before the panic");

        panic!("deliberate: the transaction must roll back on the unwind path");
    })
    .await;
    assert!(panicked.is_err(), "the spawned task panicked, as intended");

    // Close the pool so every connection's rollback is flushed and the connections
    // are gone, then ask a fresh one. Counting through the same pool would only
    // prove the rows were invisible, not that they were undone.
    before_pool.close().await;
    let fresh = pool().await;
    let after = counts(&fresh).await;

    assert_eq!(
        before, after,
        "a panicking test left rows behind. `agent_effect_log` refuses DELETE and \
         cascades that refusal up to `agent_runs`, so anything committed here is \
         permanent and every later count of agent runs is wrong by that much."
    );

    // And nothing bearing this file's markers survived, on any path.
    let marked_runs: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_runs WHERE goal = $1")
        .bind(TEST_GOAL)
        .fetch_one(&fresh)
        .await
        .expect("count marked runs");
    assert_eq!(marked_runs, 0, "a marked fixture run survived the rollback");

    let marked_users: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE email LIKE 'test-fixture-%@test.invalid'",
    )
    .fetch_one(&fresh)
    .await
    .expect("count marked users");
    assert_eq!(
        marked_users, 0,
        "a marked fixture user survived the rollback"
    );
}
