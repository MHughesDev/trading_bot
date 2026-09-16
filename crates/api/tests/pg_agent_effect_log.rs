//! The effect log's contract, against live Postgres (AGENT-010 §5; Set S S-2.5).
//!
//! Ignored by default:
//! ```bash
//! DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -p api --test pg_agent_effect_log -- --ignored --test-threads=1
//! ```
//!
//! What is asserted here is the thing `agent_messages` cannot do. A transcript can
//! tell you a tool ran; it cannot tell you whether the loop had *decided* to run it
//! before the process died, and that is the only question resume actually asks.

use sqlx::{PgPool, Row};
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://trading:trading@localhost:5432/trading".into())
}

async fn pool() -> PgPool {
    PgPool::connect(&url()).await.expect("the dev database is up")
}

/// A run to hang events off. `agent_effect_log.run_id` is a foreign key, which is the
/// point — an effect log with no run is not evidence about anything.
async fn a_run(pg: &PgPool) -> Uuid {
    // The user is created too rather than borrowed from whatever the dev database
    // happens to hold: a test that depends on a row it did not write passes or
    // fails on the state of somebody else's machine.
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user_id)
        .bind(format!("effect-log-{user_id}@test.invalid"))
        .execute(pg)
        .await
        .expect("a user row");
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_runs (run_id, user_id, goal, status, provider, model) \
         VALUES ($1, $2, 'effect-log test', 'running', 'test', 'test')",
    )
    .bind(run_id)
    .bind(user_id)
    .execute(pg)
    .await
    .expect("a run row");
    run_id
}

async fn append(pg: &PgPool, run: Uuid, seq: i32, direction: &str, kind: &str, payload: &str) {
    sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, $2, $3, $4, $5::jsonb, '0.1.0')",
    )
    .bind(run)
    .bind(seq)
    .bind(direction)
    .bind(kind)
    .bind(payload)
    .execute(pg)
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
    let pg = pool().await;
    let run = a_run(&pg).await;

    append(&pg, run, 1, "effect", "ExecuteTool",
        r#"{"step":1,"name":"create_experiment","arguments":{"slug":"ema_v1"},"idempotent":false}"#).await;
    append(&pg, run, 2, "input", "ToolResult",
        r#"{"output":"{\"experiment_id\":\"exp_1\"}","source":"platform"}"#).await;

    let rows = sqlx::query(
        "SELECT seq, direction, kind FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run)
    .fetch_all(&pg)
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
    let pg = pool().await;
    let run = a_run(&pg).await;

    append(&pg, run, 1, "effect", "CallModel", r#"{"step":1,"decode":"select_tool"}"#).await;
    append(&pg, run, 2, "input", "ModelReply", r#"{"raw":"{\"name\":\"create_experiment\"}"}"#).await;
    // Decided, and then the process died.
    append(&pg, run, 3, "effect", "ExecuteTool",
        r#"{"step":1,"name":"create_experiment","arguments":{"slug":"ema_v1"},"idempotent":false}"#).await;

    let last = sqlx::query(
        "SELECT direction, kind, payload_json FROM agent_effect_log \
         WHERE run_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(run)
    .fetch_one(&pg)
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
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn the_effect_log_refuses_to_be_edited() {
    let pg = pool().await;
    let run = a_run(&pg).await;
    append(&pg, run, 1, "effect", "CallModel", r#"{"step":1}"#).await;

    let updated = sqlx::query("UPDATE agent_effect_log SET kind = 'Tampered' WHERE run_id = $1")
        .bind(run)
        .execute(&pg)
        .await;
    assert!(updated.is_err(), "an effect log row must not be updatable");

    let deleted = sqlx::query("DELETE FROM agent_effect_log WHERE run_id = $1")
        .bind(run)
        .execute(&pg)
        .await;
    assert!(deleted.is_err(), "nor deletable");
}

/// Two rows cannot claim the same position. The order IS the content: a log whose
/// rows cannot be put back in sequence is a set of facts about a run, not the run.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn two_events_cannot_share_a_sequence_number() {
    let pg = pool().await;
    let run = a_run(&pg).await;
    append(&pg, run, 1, "effect", "CallModel", r#"{"step":1}"#).await;

    let clash = sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, 1, 'input', 'ModelReply', '{}'::jsonb, '0.1.0')",
    )
    .bind(run)
    .execute(&pg)
    .await;
    assert!(clash.is_err(), "seq is part of the key, not a hint");
}

/// `direction` is closed, so a writer cannot invent a third kind of thing and leave
/// a reader to guess whether it was a decision or an answer.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn a_row_that_is_neither_a_decision_nor_an_answer_is_refused() {
    let pg = pool().await;
    let run = a_run(&pg).await;

    let bad = sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, 1, 'sideways', 'CallModel', '{}'::jsonb, '0.1.0')",
    )
    .bind(run)
    .execute(&pg)
    .await;
    assert!(bad.is_err(), "direction is effect | input | state and nothing else");
}

/// A row with no harness version cannot be replayed, because nothing says what would
/// be replaying it. Refused rather than defaulted.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn an_event_with_no_harness_version_is_refused() {
    let pg = pool().await;
    let run = a_run(&pg).await;

    let bad = sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1, 1, 'effect', 'CallModel', '{}'::jsonb, '')",
    )
    .bind(run)
    .execute(&pg)
    .await;
    assert!(bad.is_err(), "an empty harness version is not a version");
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

    let pg = pool().await;
    let run = a_run(&pg).await;
    append(&pg, run, 1, "input", "Admission", r#"{"admission":"Admit"}"#).await;
    append(&pg, run, 2, "effect", "ExecuteTool",
        r#"{"step":1,"name":"create_experiment","arguments":{"slug":"ema_v1"},"idempotent":false}"#).await;

    let rows: Vec<(i32, String, String, serde_json::Value, String)> = sqlx::query_as(
        "SELECT seq, direction, kind, payload_json, harness_version \
         FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run)
    .fetch_all(&pg)
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

    let Resume::Replay { inputs, pending } = decide(&rows, "0.1.0") else {
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
    let pg = pool().await;
    let run = a_run(&pg).await;

    sqlx::query(
        "UPDATE agent_runs SET \
             harness_version = '0.1.0', profile_name = 'qwen2.5:7b-instruct', \
             adapter = 'ollama', transport = 'select_fill', \
             tokens_in = 1000, prompt_tokens = 40000, est_prompt_tokens = 42000, \
             cache_read_tokens = 39000, trials_spent = 4, n_eff_after = 2.5, \
             outcome = 'finished', status = 'completed' \
         WHERE run_id = $1",
    )
    .bind(run)
    .execute(&pg)
    .await
    .expect("the telemetry writes");

    let r = sqlx::query(
        "SELECT harness_version, profile_name, adapter, transport, tokens_in, \
                prompt_tokens, est_prompt_tokens, cache_read_tokens, trials_spent, \
                n_eff_after, outcome \
         FROM agent_runs WHERE run_id = $1",
    )
    .bind(run)
    .fetch_one(&pg)
    .await
    .expect("it reads back");

    assert_eq!(r.get::<String, _>("harness_version"), "0.1.0");
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
    let pg = pool().await;
    let run = a_run(&pg).await;
    let r = sqlx::query("SELECT trials_spent, outcome FROM agent_runs WHERE run_id = $1")
        .bind(run)
        .fetch_one(&pg)
        .await
        .expect("it reads back");
    assert!(r.get::<Option<i32>, _>("trials_spent").is_none());
    assert!(r.get::<Option<String>, _>("outcome").is_none());
}

/// A negative token count is not a count. Refused rather than stored.
#[tokio::test]
#[ignore = "requires live Postgres"]
async fn a_negative_token_count_is_refused() {
    let pg = pool().await;
    let run = a_run(&pg).await;
    let bad = sqlx::query("UPDATE agent_runs SET prompt_tokens = -1 WHERE run_id = $1")
        .bind(run)
        .execute(&pg)
        .await;
    assert!(bad.is_err(), "a negative prompt size is not a prompt size");
}
