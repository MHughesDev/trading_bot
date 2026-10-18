//! A session through real turns, against a fake provider (Set S phase 11:
//! S-11.1 acceptance 4, S-11.5 acceptance 2, S-11.6).
//!
//! Every other test of the session driver stops at the model: the decisions are
//! unit-tested, the stores are tested against Postgres, and **no test drove a
//! child through a real `AgentManager` turn**. That is how the lease, the drain
//! and the "bounded run has a session row" clause stayed `PARTIAL` with the
//! reason *"no test drives a real turn"*.
//!
//! This one does, with a provider that is a local HTTP server speaking the
//! Anthropic Messages wire format (`POST /v1/messages`). The whole platform side
//! is real - the manager, the run loop, the run rows, the job store, the session
//! worker - and only the model is replaced, which is exactly the seam
//! `LlmClient` already has (a credential's `base_url`). The fake answers every
//! request with a short text reply and `end_turn`, and counts how many requests
//! are in flight at once.
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/fsession_scratch \
//!   cargo test -j 2 -p api --test a_session_runs_real_turns -- --ignored --test-threads=1
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use api::agent::AgentManager;
use api::credentials::{CredentialCrypto, LlmCredentialStore};
use api::session_driver::SessionWorker;
use api::sessions;
use axum::{routing::post, Json, Router};
use jobs::store::{JobStore, PgTrialCounter, Submission};
use jobs::types::WorkerClass;
use jobs::{JobKind, WorkerPool};
use research_session::charter::{ExplorationShare, Objective, Scope, Seed};
use research_session::{Charter, EventKind, ResearchSession};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:postgres@127.0.0.1:5433/fsession_scratch".into()
    })
}

/// What the fake provider saw.
#[derive(Default)]
struct Seen {
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    requests: AtomicUsize,
}

/// A local Anthropic Messages server. `delay` keeps a request in flight long
/// enough for concurrent children to overlap in it.
async fn fake_anthropic(delay: Duration) -> (String, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let handler_seen = Arc::clone(&seen);
    let app = Router::new().route(
        "/v1/messages",
        post(move |Json(_): Json<Value>| {
            let seen = Arc::clone(&handler_seen);
            async move {
                seen.requests.fetch_add(1, Ordering::SeqCst);
                let now = seen.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                seen.peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                seen.in_flight.fetch_sub(1, Ordering::SeqCst);
                Json(json!({
                    "id": "msg_fake",
                    "type": "message",
                    "role": "assistant",
                    "model": "claude-opus-5",
                    "content": [{ "type": "text", "text": "Nothing registered yet; stopping." }],
                    "stop_reason": "end_turn",
                    "usage": { "input_tokens": 12, "output_tokens": 6 }
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
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
    profiles: Arc<harness::ProfileSet>,
}

async fn world(base: &str) -> World {
    let pg = PgPool::connect(&url()).await.expect("the database is up");
    let user = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("test-fixture-{user}@test.invalid"))
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
        AgentManager::new(
            pg.clone(),
            "http://127.0.0.1:9".into(),
            cfg::model::AgentConfig::default(),
        )
        .with_profiles(Arc::clone(&profiles)),
    );
    World {
        pg,
        user,
        manager,
        creds,
        profiles,
    }
}

async fn wait_until_over(pg: &PgPool, run: Uuid) -> String {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs DATABASE_URL migrated through 0164 and the shipped profiles"]
async fn a_bounded_research_run_is_a_session_with_one_item_through_a_real_turn() {
    let (base, seen) = fake_anthropic(Duration::from_millis(50)).await;
    let w = world(&base).await;
    let convo = api::agent::conversations::create(&w.pg, w.user, "anthropic", "claude-sonnet-5")
        .await
        .expect("a conversation");

    // The door `routes::research_sessions::create` uses for the toggle-off case.
    let run = w
        .manager
        .send_turn(
            w.user,
            convo,
            "Does cross-sectional momentum survive costs on BTC-USD?",
            &w.creds,
            api::agent::manager::TurnOptions {
                mode: Some(harness::state::Mode::Research),
                ..Default::default()
            },
        )
        .await
        .expect("the run starts against the fake provider");
    let status = wait_until_over(&w.pg, run).await;
    assert!(seen.requests.load(Ordering::SeqCst) >= 1, "the model was reached");
    eprintln!("the run ended `{status}`");

    // Acceptance 4: a session row, with one item, whose log opens and records
    // the start - written by the real front door and not by a test.
    let session = sessions::for_conversation(&w.pg, convo)
        .await
        .expect("readable")
        .expect("a Research turn opens its session row");
    assert!(!session.continuous, "a bounded run is `continuous = false`");
    let agenda = sessions::agenda(&w.pg, session.id).await.expect("agenda");
    assert_eq!(agenda.len(), 1, "the one-item case: {agenda:?}");
    let folded = sessions::state(&w.pg, session.id).await.expect("fold");
    assert!(folded.anomalies.is_empty(), "{:?}", folded.anomalies);
    assert_eq!(folded.runs_started, 1);
    let attributed: (Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT session_id, agenda_item_id FROM agent_runs WHERE run_id = $1")
            .bind(run)
            .fetch_one(&w.pg)
            .await
            .unwrap();
    assert_eq!(attributed, (Some(session.id), Some(agenda[0].id)), "the run knows its session and item");

    // A second turn in the same conversation does not open a second session.
    let again = w
        .manager
        .send_turn(
            w.user,
            convo,
            "And on ETH-USD?",
            &w.creds,
            api::agent::manager::TurnOptions {
                mode: Some(harness::state::Mode::Research),
                ..Default::default()
            },
        )
        .await
        .expect("a second turn");
    wait_until_over(&w.pg, again).await;
    let sessions_for_convo: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM research_sessions WHERE conversation_id = $1")
            .bind(convo)
            .fetch_one(&w.pg)
            .await
            .unwrap();
    assert_eq!(sessions_for_convo.0, 1, "one conversation, one session");
}

fn charter(seeds: &[&str]) -> Charter {
    Charter {
        scope: Scope {
            instruments: vec!["BTC-USD".into()],
            ..Scope::default()
        },
        objective: Objective::BuildKnowledge {
            about: vec!["BTC-USD".into()],
        },
        seeds: seeds
            .iter()
            .map(|q| Seed {
                question: (*q).to_string(),
                subject: "BTC-USD".into(),
                workflow: None,
            })
            .collect(),
        permitted_workflows: vec![],
        exclusions: vec![],
        standing_constraints: vec![],
        exploration_share: ExplorationShare::default(),
        stop_when: vec![],
        delta_practical_default: None,
        version: 1,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs DATABASE_URL migrated through 0164 and the shipped profiles"]
async fn three_children_run_together_and_their_leases_never_exceed_the_day_in_trials() {
    // Each model call takes long enough that three children overlap in it.
    let (base, seen) = fake_anthropic(Duration::from_millis(900)).await;
    let w = world(&base).await;
    let journal = api::agent::conversations::create(&w.pg, w.user, "anthropic", "claude-sonnet-5")
        .await
        .expect("a journal");

    let mut session = ResearchSession::continuous(
        Uuid::new_v4(),
        journal,
        charter(&[
            "does momentum survive costs on BTC-USD?",
            "is the funding carry on BTC-USD stable?",
            "does the weekend effect hold on BTC-USD?",
        ]),
        "claude-opus-5",
    );
    session.schedule.max_concurrent_runs = 3;
    // 40 trials a child (`trials_total / max_runs`), and a day of 90: three
    // children cannot all be leased 40 - the third is cut to what remains.
    session.budget.trials_total = 400;
    session.budget.max_runs = 10;
    sessions::create(&w.pg, w.user, &session).await.expect("the session");
    api::session_day::set_trials_per_day(&w.pg, session.id, 90)
        .await
        .expect("a day budget");
    sessions::append(&w.pg, session.id, EventKind::SessionStarted, json!({}))
        .await
        .expect("started");

    let store = Arc::new(JobStore::new(w.pg.clone(), Arc::new(PgTrialCounter)));
    let worker: Arc<dyn jobs::Worker> = Arc::new(SessionWorker::new(
        w.pg.clone(),
        Arc::clone(&w.manager),
        w.creds.clone(),
        Arc::clone(&w.profiles),
    ));
    let submitted = store
        .submit(Submission::new(
            JobKind::ContinuousResearch,
            w.user,
            json!({ "session_id": session.id, "user_id": w.user }),
        ))
        .await
        .expect("the driver job");
    // A scratch database keeps what earlier runs left queued; claim past it.
    let claimed = loop {
        let c = store
            .claim(WorkerClass::Research, "test-worker")
            .await
            .expect("claim")
            .expect("the job just submitted");
        if c.job_id == submitted.job_id {
            break c;
        }
    };
    WorkerPool::new(Arc::clone(&store), WorkerClass::Research, 1)
        .register(worker)
        .execute(claimed)
        .await;

    // S-11.5/6: the children really overlapped in the provider.
    let peak = seen.peak.load(Ordering::SeqCst);
    assert!(peak >= 2, "children were started one at a time: peak {peak} in flight");

    // §12.4: leases, summed. Every lease was issued in trials, the three
    // together fit the day, and the one that did not fit was cut by the day.
    type Lease = (
        f64,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    );
    let leases: Vec<Lease> = sqlx::query_as(
        "SELECT size, unit, capped_by, issued_at, released_at FROM session_leases
          WHERE session_id = $1 ORDER BY issued_at",
    )
    .bind(session.id)
    .fetch_all(&w.pg)
    .await
    .unwrap();
    assert!(leases.len() >= 3, "{leases:?}");
    assert!(leases.iter().all(|l| l.1 == "trials"), "{leases:?}");
    // At the instant each lease was issued, everything still committed - itself
    // included - fits the day. That is "three concurrent children cannot together
    // exceed the budget" as a property of the rows and not of the timing.
    for l in &leases {
        let committed: f64 = leases
            .iter()
            .filter(|o| o.3 <= l.3 && o.4.is_none_or(|r| r > l.3))
            .map(|o| o.0)
            .sum();
        assert!(committed <= 90.0 + 1e-9, "{committed} committed at {}: {leases:?}", l.3);
    }
    let overlapping = leases
        .iter()
        .filter(|l| leases.iter().filter(|o| o.3 <= l.3 && o.4.is_none_or(|r| r > l.3)).count() >= 2)
        .count();
    assert!(overlapping >= 1, "no two leases were ever committed together: {leases:?}");

    // And the log is whole: every child started and finished, nothing anomalous.
    let folded = sessions::state(&w.pg, session.id).await.expect("fold");
    assert!(folded.anomalies.is_empty(), "{:?}", folded.anomalies);
    assert!(folded.runs_started >= 3, "{}", folded.runs_started);
}
