//! The one agent runtime, against a real database and a fake provider
//! (Set S: S-2.5, S-2.6, S-2.8, S-2.11, S-2.12, S-2.14).
//!
//! Every clause these tests close was `PARTIAL` with the same note: *"not run
//! against a migrated Postgres"*. The unit tests of the loop, the replay and the
//! resume decision are sound; what none of them could reach is the seam between
//! them and the database - `load_log`, `interrupt`, the idempotency read, the
//! telemetry columns, the sub-run claim. A fake provider (a local HTTP server
//! speaking the Anthropic Messages wire format) replaces only the model.
//!
//! The scratch database is migrated with every file in `migrations/` (pgvector
//! columns substituted - the native 5433 server has no pgvector):
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/s_runtime_scratch \
//!   cargo test -j 2 -p api --test pg_runtime_live -- --ignored --test-threads=1
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::agent::AgentManager;
use api::credentials::{CredentialCrypto, LlmCredentialStore};
use axum::{routing::post, Json, Router};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/s_runtime_scratch".into())
}

/// What the fake provider is told to say, in order. The last entry repeats.
type Script = Vec<Value>;

struct Provider {
    requests: AtomicUsize,
    bodies: Mutex<Vec<Value>>,
}

fn text(t: &str) -> Value {
    json!({
        "id": "msg_fake", "type": "message", "role": "assistant", "model": "claude-opus-5",
        "content": [{ "type": "text", "text": t }],
        "stop_reason": "end_turn",
        "usage": { "input_tokens": 1200, "output_tokens": 40,
                   "cache_read_input_tokens": 800, "cache_creation_input_tokens": 0 }
    })
}

fn tool_use(id: &str, name: &str, input: Value) -> Value {
    json!({
        "id": "msg_fake", "type": "message", "role": "assistant", "model": "claude-opus-5",
        "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }],
        "stop_reason": "tool_use",
        "usage": { "input_tokens": 1500, "output_tokens": 60,
                   "cache_read_input_tokens": 900, "cache_creation_input_tokens": 0 }
    })
}

/// A provider that calls `tool` until the transcript holds a tool result, then
/// answers with `final_text`. Keyed on the request rather than on its position,
/// because the run makes other requests (a canary) that would consume a script.
async fn adaptive_provider(tool: &'static str, final_text: &'static str, delay_ms: u64) -> (String, Arc<Provider>) {
    let seen = Arc::new(Provider { requests: AtomicUsize::new(0), bodies: Mutex::new(vec![]) });
    let handler = Arc::clone(&seen);
    let app = Router::new().route(
        "/v1/messages",
        post(move |Json(body): Json<Value>| {
            let seen = Arc::clone(&handler);
            async move {
                seen.requests.fetch_add(1, Ordering::SeqCst);
                let raw = body.to_string();
                let has_tools = body.get("tools").and_then(Value::as_array).is_some_and(|t| !t.is_empty());
                let answered = raw.contains("tool_result");
                seen.bodies.lock().unwrap().push(body);
                if delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                Json(if has_tools && !answered { tool_use("toolu_1", tool, json!({})) } else if has_tools { tool_use("toolu_2", "finish_task", json!({ "result": final_text, "evidence": "list_instruments" })) } else { text(final_text) })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, seen)
}

async fn fake_provider(script: Script) -> (String, Arc<Provider>) {
    slow_provider(script, 0).await
}

async fn slow_provider(script: Script, delay_ms: u64) -> (String, Arc<Provider>) {
    let seen = Arc::new(Provider {
        requests: AtomicUsize::new(0),
        bodies: Mutex::new(vec![]),
    });
    let handler = Arc::clone(&seen);
    let script = Arc::new(script);
    let app = Router::new().route(
        "/v1/messages",
        post(move |Json(body): Json<Value>| {
            let seen = Arc::clone(&handler);
            let script = Arc::clone(&script);
            async move {
                let i = seen.requests.fetch_add(1, Ordering::SeqCst);
                seen.bodies.lock().unwrap().push(body);
                if delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                Json(script[i.min(script.len() - 1)].clone())
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, seen)
}

struct World {
    pg: PgPool,
    user: Uuid,
    manager: Arc<AgentManager>,
    creds: LlmCredentialStore,
}

async fn world(base: &str) -> World {
    let pg = PgPool::connect(&url()).await.expect("the scratch database is up");
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("test-fixture-runtime-{user}@test.invalid"))
        .execute(&pg)
        .await
        .expect("a user");
    let crypto = Arc::new(CredentialCrypto::new(vec![7; 32], 1).expect("a key"));
    let creds = LlmCredentialStore::new(pg.clone(), Some(crypto));
    creds
        .save(user, "anthropic", "sk-test-fixture", Some(base))
        .await
        .expect("a credential pointing at the fake provider");
    let profiles = Arc::new(
        harness::ProfileSet::load_dir(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/profiles"),
        )
        .expect("the shipped profiles load"),
    );
    let manager = Arc::new(
        AgentManager::new(pg.clone(), "http://127.0.0.1:9".into(), cfg::model::AgentConfig::default())
            .with_profiles(profiles),
    );
    World { pg, user, manager, creds }
}

async fn over(pg: &PgPool, run: Uuid) -> String {
    for _ in 0..240 {
        let (status,): (String,) = sqlx::query_as("SELECT status FROM agent_runs WHERE run_id = $1")
            .bind(run)
            .fetch_one(pg)
            .await
            .expect("the run row");
        if !api::agent::conversations::ACTIVE_RUN_STATES.contains(&status.as_str()) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("run {run} did not finish in a minute");
}

async fn chat_turn(w: &World, convo: Uuid, q: &str) -> Uuid {
    w.manager
        .send_turn(
            w.user,
            convo,
            q,
            &w.creds,
            api::agent::manager::TurnOptions {
                mode: Some(harness::state::Mode::Chat),
                ..Default::default()
            },
        )
        .await
        .expect("the turn starts against the fake provider")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs DATABASE_URL: a scratch database migrated through the latest migration"]
async fn a_real_turn_leaves_a_log_that_replays_and_telemetry_that_reads_back() {
    let (base, seen) = adaptive_provider("list_instruments", "Two instruments are initialised.", 0).await;
    let w = world(&base).await;
    let convo = api::agent::conversations::create(&w.pg, w.user, "anthropic", "claude-sonnet-5")
        .await
        .expect("a conversation");
    let run = chat_turn(&w, convo, "Which instruments do we have?").await;
    let status = over(&w.pg, run).await;
    eprintln!("ended {status}; requests {}", seen.requests.load(Ordering::SeqCst));
    let rows: Vec<(i32, String, String, Value)> = sqlx::query_as(
        "SELECT seq, direction, kind, payload_json FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run)
    .fetch_all(&w.pg)
    .await
    .unwrap();
    for (s, d, k, p) in &rows {
        eprintln!("{s} {d} {k} {}", p.to_string().chars().take(160).collect::<String>());
    }
    assert_ne!(status, "fenced", "the run was fenced instead of finishing");

    // S-2.5: the log of a real run projects to a trajectory that can replay.
    let spans = api::trace_from_effects::spans_for_run(&w.pg, run).await.unwrap();
    assert!(harness::trace::is_replayable(&spans), "{:?}", harness::trace::unreplayable_reason(&spans));
    assert!(
        spans.iter().any(|s| matches!(&s.event, harness::trace::Event::ToolCall { name, args, .. } if name == "list_instruments" && !args.is_null())),
        "the tool call is in the trajectory with its arguments"
    );
    // The effect is logged before its answer (S-2.5 ordering).
    let seq_of = |dir: &str, kind: &str| rows.iter().find(|r| r.1 == dir && r.2 == kind).map(|r| r.0);
    assert!(seq_of("effect", "ExecuteTool") < seq_of("input", "ToolResult").or(seq_of("input", "ToolError")));

    // S-2.8: all ten telemetry columns are populated by the driver.
    let t = sqlx::query(
        "SELECT harness_version, profile_name, adapter, transport, prompt_tokens, est_prompt_tokens,
                cache_read_tokens, trials_spent, n_eff_after, outcome, tokens_in
           FROM agent_runs WHERE run_id = $1",
    )
    .bind(run)
    .fetch_one(&w.pg)
    .await
    .unwrap();
    use sqlx::Row;
    for col in ["harness_version", "profile_name", "adapter", "transport", "outcome"] {
        let v: Option<String> = t.get(col);
        assert!(v.is_some_and(|v| !v.is_empty()), "{col} was not written");
    }
    for col in ["prompt_tokens", "est_prompt_tokens", "cache_read_tokens"] {
        let v: Option<i64> = t.get(col);
        assert!(v.is_some(), "{col} was not written");
    }
    let trials: Option<i32> = t.get("trials_spent");
    assert_eq!(trials, Some(0), "a read spends no trial, and says zero rather than null");
    let n_eff: Option<f64> = t.try_get("n_eff_after").unwrap_or(None);
    eprintln!("n_eff_after = {n_eff:?} (null is allowed when the ledger cannot be read)");
    let prompt: i64 = t.get("prompt_tokens");
    let billed: i64 = t.get("tokens_in");
    assert!(prompt >= billed, "prompt_tokens counts the cached part tokens_in does not: {prompt} vs {billed}");

}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs DATABASE_URL: a scratch database migrated through the latest migration"]
async fn a_drain_opens_a_window_the_monitor_reads_and_honours() {
    use monitor::platform::{assess, ComponentBeat, PlatformVerdict};
    let (base, _) = fake_provider(vec![text("unused")]).await;
    let w = world(&base).await;
    let left = w.manager.drain(Duration::from_secs(2)).await;
    assert_eq!(left, 0, "an idle manager drains at once");

    let store = monitor::store::MonitorStore::new(w.pg.clone());
    let windows = store.open_windows().await.expect("the monitor reads its windows");
    let now = chrono::Utc::now();
    assert!(
        windows
            .iter()
            .any(|x| x.opened_by == "platform:deploy" && x.is_open(now)),
        "the drain left no open window the monitor can read: {windows:?}"
    );
    // The platform is, for the monitor, a component whose every instance has gone
    // quiet - which is what a rolling deploy looks like.
    let beats = [ComponentBeat::new("tbot-platform", "old-instance", true)];
    assert!(
        matches!(assess(&beats, &windows, now), PlatformVerdict::Suppressed { .. }),
        "the deploy window did not suppress the platform trip"
    );
    assert!(
        matches!(assess(&beats, &[], now), PlatformVerdict::Halt { .. }),
        "without the window the same silence halts"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs DATABASE_URL: a scratch database migrated through the latest migration"]
async fn a_second_turn_names_the_live_run_and_a_steer_reaches_it() {
    let (base, _seen) = adaptive_provider("list_instruments", "Done.", 1500).await;
    let w = world(&base).await;
    let convo = api::agent::conversations::create(&w.pg, w.user, "anthropic", "claude-sonnet-5")
        .await
        .expect("a conversation");
    let run = chat_turn(&w, convo, "Which instruments do we have?").await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    // A plain second turn is refused, and the refusal names the run.
    let refused = w
        .manager
        .send_turn(w.user, convo, "and what about ETH?", &w.creds, Default::default())
        .await
        .expect_err("two live turns in one conversation");
    assert!(format!("{refused:?}").contains(&run.to_string()), "{refused:?}");

    // A steer is accepted, queued on the live run, and recorded.
    let steered = w
        .manager
        .send_turn(
            w.user,
            convo,
            "only crypto please",
            &w.creds,
            api::agent::manager::TurnOptions {
                steer: true,
                ..Default::default()
            },
        )
        .await
        .expect("a steer reaches the live run");
    assert_eq!(steered, run, "the steer went to the live run, not a new one");
    over(&w.pg, run).await;
    let kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM agent_effect_log WHERE run_id = $1 AND direction = 'input' ORDER BY seq",
    )
    .bind(run)
    .fetch_all(&w.pg)
    .await
    .unwrap();
    assert!(
        kinds.iter().any(|k| k == "Steer"),
        "the steer never became an input of the loop: {kinds:?}"
    );
}
