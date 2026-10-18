//! `spans_for_run` and the replayability sampler against a real `agent_effect_log`
//! (S-4.8).
//!
//! The three unit tests in `trace_from_effects.rs` feed `event_for` hand-built
//! JSON; none touched the query, and `spans_for_run` had no caller. These do.
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test a_trajectory_is_read_from_the_effect_log -- --ignored --test-threads=1
//! ```

use api::trace_from_effects;

use harness::trace::Event;
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

async fn db() -> PgPool {
    let mut admin = PgConnection::connect(&with_db(&base_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute("DROP DATABASE IF EXISTS trajectory_test WITH (FORCE)")
        .await
        .unwrap();
    admin
        .execute("CREATE DATABASE trajectory_test")
        .await
        .unwrap();
    let pool = PgPool::connect(&with_db(&base_url(), "trajectory_test"))
        .await
        .unwrap();
    // The two things 0061 needs that are not 0061: the table it references and
    // the trigger function it installs.
    pool.execute(
        "CREATE SCHEMA mlops; \
         CREATE FUNCTION mlops.refuse_mutation() RETURNS trigger AS \
           $$ BEGIN RAISE EXCEPTION 'append-only'; END $$ LANGUAGE plpgsql; \
         CREATE TABLE agent_runs (run_id UUID PRIMARY KEY);",
    )
    .await
    .unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("two below the root")
        .join("migrations/0061_agent_effect_log.sql");
    let sql = std::fs::read_to_string(path).expect("0061 readable");
    pool.execute(sql.as_str()).await.expect("0061 applies");
    pool
}

async fn log(
    pool: &PgPool,
    run: Uuid,
    seq: i32,
    direction: &str,
    kind: &str,
    payload: serde_json::Value,
) {
    sqlx::query(
        "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version) \
         VALUES ($1,$2,$3,$4,$5,'test')",
    )
    .bind(run)
    .bind(seq)
    .bind(direction)
    .bind(kind)
    .bind(payload)
    .execute(pool)
    .await
    .unwrap();
}

async fn a_run(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_runs (run_id) VALUES ($1)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    id
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_trajectory_is_the_effect_rows_in_order_with_arguments_intact() {
    let pool = db().await;
    let run = a_run(&pool).await;
    // Written out of order on purpose: the ORDER BY seq is what is under test.
    log(&pool, run, 3, "effect", "Note", serde_json::json!({"core_tool": "record_finding", "args": {"finding": "x"}, "detail": "stored"})).await;
    log(
        &pool,
        run,
        1,
        "effect",
        "CallModel",
        serde_json::json!({"prompt_bytes": 2048, "step": 1}),
    )
    .await;
    log(&pool, run, 2, "effect", "ExecuteTool", serde_json::json!({"name": "read_bars", "namespace": "data", "arguments": {"instrument": "BTC-USD"}, "step": 2})).await;
    log(
        &pool,
        run,
        4,
        "input",
        "ToolResult",
        serde_json::json!({"output": "x"}),
    )
    .await;

    let spans = trace_from_effects::spans_for_run(&pool, run).await.unwrap();
    assert_eq!(spans.len(), 3, "the input row is not projected");
    assert!(matches!(spans[0].event, Event::ModelCall { .. }));
    match &spans[1].event {
        Event::ToolCall { name, args, .. } => {
            assert_eq!(name, "read_bars");
            assert_eq!(
                args["instrument"], "BTC-USD",
                "arguments survive the round trip"
            );
        }
        other => panic!("expected a tool call, got {other:?}"),
    }
    assert!(matches!(spans[2].event, Event::CoreTool { .. }));
    assert_eq!(spans[1].step, 2);
    assert_eq!(
        spans[2].step, 2,
        "a row with no step carries the previous one forward"
    );
    assert!(harness::trace::is_replayable(&spans));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn the_sampler_counts_replayable_runs_and_names_the_ones_that_are_not() {
    let pool = db().await;
    let good = a_run(&pool).await;
    log(
        &pool,
        good,
        1,
        "effect",
        "CallModel",
        serde_json::json!({"prompt_bytes": 10, "step": 1}),
    )
    .await;
    log(
        &pool,
        good,
        2,
        "effect",
        "ExecuteTool",
        serde_json::json!({"name": "read_bars", "arguments": {"instrument": "ETH-USD"}}),
    )
    .await;
    // Tool name logged, arguments lost: the shape is_replayable exists to catch.
    let bad = a_run(&pool).await;
    log(
        &pool,
        bad,
        1,
        "effect",
        "CallModel",
        serde_json::json!({"prompt_bytes": 10}),
    )
    .await;
    log(
        &pool,
        bad,
        2,
        "effect",
        "ExecuteTool",
        serde_json::json!({"name": "read_bars"}),
    )
    .await;

    let sample = trace_from_effects::sample_replayable(&pool, 10)
        .await
        .unwrap();
    assert_eq!(sample.sampled, 2);
    assert_eq!(sample.replayable, 1);
    assert_eq!(sample.unreplayable.len(), 1);
    assert_eq!(sample.unreplayable[0].0, bad);
    assert!(!sample.unreplayable[0].1.is_empty(), "the reason is stated");
}

/// A finding as production now writes it is replayable (S-4.8).
///
/// `describe_effect` writes a core-tool `Note` as `{core_tool, detail, args}`, so
/// the model's arguments to `record_finding` / `finish_task` are on the row and a
/// run made of findings replays. This was pinned the other way ("not replayable
/// yet") while `harness::Note::CoreTool` carried no `args`; it now holds the
/// closed gap, and the legacy shape is kept below so a row written before the
/// change is still reported honestly.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_core_tool_note_as_production_writes_it_is_replayable() {
    let pool = db().await;
    let run = a_run(&pool).await;
    log(
        &pool,
        run,
        1,
        "effect",
        "CallModel",
        serde_json::json!({"prompt_bytes": 10, "step": 1}),
    )
    .await;
    log(
        &pool,
        run,
        2,
        "effect",
        "Note",
        serde_json::json!({"core_tool": "record_finding", "detail": "stored",
                           "args": {"finding": "BTC-USD momentum", "evidence": "exp_1"}}),
    )
    .await;
    let sample = trace_from_effects::sample_replayable(&pool, 5)
        .await
        .unwrap();
    assert_eq!(sample.replayable, 1, "{:?}", sample.unreplayable);
}

/// A row written before `args` existed has a tool name and no arguments, which
/// the projection does not invent, so it is still reported unreplayable.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires live Postgres"]
async fn a_legacy_core_tool_note_without_args_is_still_unreplayable() {
    let pool = db().await;
    let run = a_run(&pool).await;
    log(
        &pool,
        run,
        1,
        "effect",
        "CallModel",
        serde_json::json!({"prompt_bytes": 10, "step": 1}),
    )
    .await;
    log(
        &pool,
        run,
        2,
        "effect",
        "Note",
        serde_json::json!({"core_tool": "record_finding", "detail": "stored"}),
    )
    .await;
    let sample = trace_from_effects::sample_replayable(&pool, 5)
        .await
        .unwrap();
    assert_eq!(sample.replayable, 0);
    assert!(
        sample.unreplayable[0].1.contains("record_finding"),
        "{:?}",
        sample.unreplayable
    );
}
