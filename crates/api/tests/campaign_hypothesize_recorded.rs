//! S-10.11: a campaign's HYPOTHESIZE runs through the worker sub-run, over a
//! **recorded transport** (AGENT-013 §8.5; ADR-S10-06, ADR-S10-16).
//!
//! The acceptance asks for "a campaign advancing through HYPOTHESIZE via the worker
//! sub-run on the reference fixture". The live half - a real model choosing the
//! move - needs a model server and a credential and is gate H-2. Everything the
//! platform does around the model does not: the pinned brief, the claim, the
//! lease, the grammar, the single retry with the validator's reason, the settled
//! outcome and the propensity. This test runs all of it against a local server that
//! replays canned Ollama replies, so the only thing it does not exercise is the
//! model's own judgement.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p api --test campaign_hypothesize_recorded -- --ignored --test-threads=1
//! ```
//!
//! The scratch database needs migrations through 0171 (pgvector is not needed).

use std::sync::{Arc, Mutex};

use api::agent::subrun;
use api::credentials::{CredentialCrypto, LlmCredentialStore};
use api::research_run_worker::ResearchRunWorker;
use jobs::store::{JobStore, PgTrialCounter, Submission};
use jobs::types::WorkerClass;
use jobs::{JobKind, WorkerPool};
use ledger::campaign::{Budget, CampaignDefinition, ExplorationFloor};
use ledger::phase::{CampaignEvent, CampaignPhase};
use ledger::pg::PgTrialLedger;
use serde_json::{json, Value};
use sqlx::{Executor, PgPool};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

async fn pool() -> PgPool {
    PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

/// A local server that answers each `POST /api/chat` with the next canned
/// content, recording every request body it was sent.
struct Recorded {
    base_url: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

async fn recorded_ollama(replies: Vec<String>) -> Recorded {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let replies = Arc::new(Mutex::new(replies.into_iter()));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let (seen, replies) = (Arc::clone(&seen), Arc::clone(&replies));
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (header_end, content_length) = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..at]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (at + 4, len);
                    }
                };
                while buf.len() < header_end + content_length {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body: Value =
                    serde_json::from_slice(&buf[header_end..]).unwrap_or(Value::Null);
                seen.lock().unwrap().push(body);
                let content = replies.lock().unwrap().next().unwrap_or_default();
                let reply = json!({
                    "message": { "role": "assistant", "content": content },
                    "done": true,
                    "done_reason": "stop",
                    "prompt_eval_count": 120,
                    "eval_count": 40
                })
                .to_string();
                let _ = sock
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                            reply.len()
                        )
                        .as_bytes(),
                    )
                    .await;
            });
        }
    });
    Recorded { base_url, requests }
}

fn definition(slug: &str) -> CampaignDefinition {
    CampaignDefinition {
        slug: slug.into(),
        hypothesis: "a faster fast window earns more after costs".into(),
        objective: json!({ "maximize": ["sharpe"], "subject_to": {} }),
        benchmark: json!({ "kind": "declared_incumbent" }),
        delta_practical: 0.15,
        approval_spend_usd: 5.0,
        budget: Budget {
            max_trials: 5_000,
            gpu_hours: 1.0,
            usd: 10.0,
            wall_clock_hours: 24.0,
        },
        exploration_floor: ExplorationFloor::default_floor(),
        gates_profile: "strict_v1".into(),
        preference_vector: None,
        search_space: json!({}),
    }
}

const BRIEF: &str = "REGISTRATION: maximise sharpe after costs on BTC-USD 1h; \
                     falsifier: sharpe <= 0 out of sample.\nFINDINGS SO FAR: none.";

struct World {
    pg: PgPool,
    store: Arc<JobStore>,
    user: Uuid,
    campaign_id: Uuid,
    driver_job: String,
    sub_run_id: Uuid,
}

/// A user whose parent run is on the `ollama` provider pointing at `base_url`, a
/// defined campaign with the log folded to just before HYPOTHESIZE, its pinned
/// brief, and the claimed sub-run position.
async fn world(base_url: &str) -> World {
    let pg = pool().await;
    let user = Uuid::new_v4();
    let tenant = user.to_string();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("{}@test.invalid", user.simple()))
        .execute(&pg)
        .await
        .expect("a user");
    let parent = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_runs (run_id, user_id, goal, provider, model)
         VALUES ($1, $2, 'optimise the ema cross', 'ollama', 'claude-opus-5')",
    )
    .bind(parent)
    .bind(user)
    .execute(&pg)
    .await
    .expect("the parent run");

    // The platform's own credential path: encrypted, with the endpoint override
    // that points the client at the recorded server.
    let crypto = Arc::new(CredentialCrypto::new(vec![7u8; 32], 1).expect("a kek"));
    LlmCredentialStore::new(pg.clone(), Some(crypto))
        .save(user, "ollama", "", Some(base_url))
        .await
        .expect("credential saved");

    let led = PgTrialLedger::new(pg.clone());
    let slug = format!("hyp-{}", Uuid::new_v4().simple());
    let handle = led
        .define_campaign_async(&tenant, &definition(&slug))
        .await
        .expect("define");
    let campaign_id = handle.campaign_id();
    let store = Arc::new(JobStore::new(pg.clone(), Arc::new(PgTrialCounter)));
    let mut driver = Submission::new(JobKind::Campaign, user, json!({ "campaign_id": campaign_id }));
    driver.project_id = Some(Uuid::new_v4());
    let driver_job = store.submit(driver).await.expect("driver job").job_id;
    for phase in [CampaignPhase::Baseline, CampaignPhase::Diagnose] {
        led.append_campaign_event_async(&tenant, campaign_id, &CampaignEvent::new(phase, json!({})))
            .await
            .expect("append");
    }

    // The campaign's row, written the way `record_campaign` writes it, with the
    // registration FK waived for this fixture alone (the registration is not what
    // is under test, and its columns are another migration's business).
    let digest = subrun::digest(BRIEF);
    let mut c = pg.acquire().await.unwrap();
    c.execute("SET session_replication_role = replica").await.unwrap();
    sqlx::query(
        "INSERT INTO agent_lifecycle.campaign_run
             (campaign_id, run_id, tenant_id, prereg_id, prereg_hash, job_id,
              projection, brief, brief_digest, shown, withheld)
         VALUES ($1, $2, $3, $4, 'sha256:prereg', $5, 'worker', $6, $7, '[]'::jsonb, 3)",
    )
    .bind(campaign_id)
    .bind(parent)
    .bind(&tenant)
    .bind(Uuid::new_v4())
    .bind(&driver_job)
    .bind(BRIEF)
    .bind(&digest)
    .execute(&mut *c)
    .await
    .expect("campaign_run");
    drop(c);
    let brief = subrun::campaign_brief(&pg, campaign_id)
        .await
        .unwrap()
        .expect("the pinned brief");
    let claim = subrun::claim(&pg, campaign_id, parent, &tenant, "hypothesize", 3, &brief, 4)
        .await
        .expect("claimed");

    World {
        pg,
        store,
        user,
        campaign_id,
        driver_job,
        sub_run_id: claim.sub_run_id,
    }
}

async fn run_hypothesize(w: &World) -> (String, Value) {
    let profiles = Arc::new(
        harness::ProfileSet::load_dir(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/profiles"))
            .expect("profiles load"),
    );
    let worker = Arc::new(ResearchRunWorker::new(
        Arc::clone(&w.store),
        w.pg.clone(),
        Some(Arc::new(CredentialCrypto::new(vec![7u8; 32], 1).unwrap())),
        profiles,
        "claude-opus-5".into(),
    ));
    let mut sub = Submission::new(
        JobKind::ResearchRun,
        w.user,
        json!({
            "campaign_id": w.campaign_id,
            "phase": "hypothesize",
            "seq": 3,
            "sub_run_id": w.sub_run_id,
        }),
    );
    sub.parent_job_id = Some(w.driver_job.clone());
    sub.member_index = Some(3);
    let submitted = w.store.submit_child_once(sub).await.expect("child");
    // The scratch database is shared, so other queued research jobs may be ahead
    // of this one; claim until it is ours.
    let claimed = loop {
        let c = w
            .store
            .claim(WorkerClass::Research, "test-worker")
            .await
            .expect("claim")
            .expect("our queued child is in the queue");
        if c.job_id == submitted.job_id {
            break c;
        }
    };
    WorkerPool::new(Arc::clone(&w.store), WorkerClass::Research, 1)
        .register(worker)
        .execute(claimed)
        .await;
    let job = w.store.get(&submitted.job_id).await.unwrap();
    (
        job.state.as_str().to_string(),
        job.result.unwrap_or_else(|| json!({ "error": job.error })),
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0171"]
async fn a_campaign_hypothesizes_through_the_worker_sub_run() {
    let server = recorded_ollama(vec![json!({
        "move": "simplify",
        "hypothesis": "dropping the least useful node costs nothing after costs",
        "ranges": []
    })
    .to_string()])
    .await;
    let w = world(&server.base_url).await;

    let (state, result) = run_hypothesize(&w).await;
    assert_eq!(state, "succeeded", "{result}");

    // The proposal is typed, validated, and says what check it still owes.
    assert_eq!(result["proposal"]["move"], "simplify");
    assert_eq!(result["role"], "worker");
    assert_eq!(result["projection"], "worker");
    assert_eq!(result["propensity"], 1.0, "temperature pinned to zero, so the propensity is knowable");
    assert!(result["candidate_set"].as_array().unwrap().len() >= 8, "every move the vocabulary offered");

    // Exactly one call went out, at temperature zero, under the grammar, and the
    // message **starts with the pinned brief byte for byte**: that prefix is what
    // every worker of the campaign shares and what the provider cache keys on.
    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "one model call");
    let sent = &requests[0];
    assert_eq!(sent["options"]["temperature"], 0.0, "{sent}");
    assert!(sent["format"].is_object(), "a grammar was sent: {sent}");
    let user_message = sent["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .expect("a user message");
    assert!(
        user_message["content"].as_str().unwrap().starts_with(BRIEF),
        "the pinned brief is the prefix"
    );

    // The sub-run's row records the outcome, so the claim cannot be taken twice.
    let outcome: Option<Value> = sqlx::query_scalar(
        "SELECT outcome FROM agent_lifecycle.sub_run WHERE sub_run_id = $1",
    )
    .bind(w.sub_run_id)
    .fetch_one(&w.pg)
    .await
    .unwrap();
    assert_eq!(outcome.expect("settled")["proposal"]["move"], "simplify");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0171"]
async fn a_refused_proposal_gets_one_retry_carrying_the_validators_reason() {
    let server = recorded_ollama(vec![
        // Not a proposal at all.
        "I think we should try something faster.".to_string(),
        json!({
            "move": "change_exit",
            "mode": "trail",
            "hypothesis": "a trailing exit keeps the winners",
            "ranges": []
        })
        .to_string(),
    ])
    .await;
    let w = world(&server.base_url).await;

    let (state, result) = run_hypothesize(&w).await;
    assert_eq!(state, "succeeded", "{result}");
    assert_eq!(result["proposal"]["move"], "change_exit");

    let requests = server.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "exactly one retry");
    let second = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "user")
        .unwrap()["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(second.starts_with(BRIEF), "the brief is still the prefix on the retry");
    assert!(
        second.contains("YOUR LAST ANSWER WAS REFUSED"),
        "the retry names why, after the shared prefix"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0171"]
async fn two_refusals_end_the_generation_with_a_typed_reason_and_no_third_call() {
    let server = recorded_ollama(vec!["not json".to_string(), "still not json".to_string()]).await;
    let w = world(&server.base_url).await;
    let (state, result) = run_hypothesize(&w).await;
    assert_eq!(state, "failed", "{result}");
    assert_eq!(server.requests.lock().unwrap().len(), 2, "two attempts, never a third");
}
