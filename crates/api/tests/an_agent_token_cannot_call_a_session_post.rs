//! S-11.14's adversarial clause, over HTTP: **an agent token cannot call any
//! session write**, and can still read.
//!
//! Until 2026-10-04 this was asserted only by a source scan
//! (`every_mutating_handler_asks_the_same_question`): a scan proves a line of
//! text is present, not that a request carrying an agent's credential is
//! refused. This test sends the request.
//!
//! The platform's own router and `AppState`, built in process the way
//! `apps/platform/src/main.rs` builds them (nothing is spawned). Two agent
//! credentials are minted the way the platform mints them:
//!
//! * the **project-bound research token**, through
//!   `SessionOrchestrator::mint_token` (scopes = `RESEARCH_SCOPES`);
//! * the **run token** the agent manager gives an internal agent run
//!   (`kind = 'service'`, label `agent-run-<uuid>`, the column's default scopes;
//!   the SQL is `AgentManager::mint_run_token`'s, which is private).
//!
//! and each is sent to **every** mutating route of
//! `routes/research_sessions.rs`. For each: the answer is `403
//! not_a_human_session`, and a before/after digest of every table a session
//! write could touch is unchanged. A human (`web:full`) token sent the same
//! request is *not* refused, so the 403 is the scope and not a malformed
//! request. The same agent tokens read the session, board, digests and events.
//!
//! Needs a Postgres that can create databases. Default
//! `postgres://postgres:postgres@127.0.0.1:5433/postgres` (never 5432, which is
//! not the platform's database); override with `SESSION_TEST_PG_URL`. A scratch
//! database is made, migrated and dropped. When no server answers the test
//! prints `SKIPPED` and passes: that is the one case it does not run.
//!
//! ```text
//! cargo test -j 2 -p api --test an_agent_token_cannot_call_a_session_post
//! ```

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

fn base_url() -> String {
    std::env::var("SESSION_TEST_PG_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/postgres".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

async fn scratch() -> Option<(PgPool, String)> {
    let name = format!("agent_post_{}", Uuid::new_v4().simple());
    // The server on 5433 is shared and restarts now and then: wait for it before
    // concluding it is not there.
    let mut admin = None;
    for _ in 0..20 {
        if let Ok(c) = PgConnection::connect(&with_db(&base_url(), "postgres")).await {
            admin = Some(c);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
    let mut admin = admin?;
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .expect("CREATE DATABASE");
    let pg = PgPool::connect(&with_db(&base_url(), &name))
        .await
        .expect("scratch connect");
    let mut skipped = Vec::new();
    for mig in sqlx::migrate!("../../migrations").iter() {
        if mig.migration_type.is_down_migration() {
            continue;
        }
        let mut tx = pg.begin().await.unwrap();
        match tx.execute(&*mig.sql).await {
            Ok(_) => tx.commit().await.unwrap(),
            Err(_) => {
                tx.rollback().await.ok();
                skipped.push(mig.version);
            }
        }
    }
    if !skipped.is_empty() {
        eprintln!("scratch database skipped migrations {skipped:?}");
    }
    Some((pg, name))
}

async fn drop_db(pg: PgPool, name: &str) {
    pg.close().await;
    if let Ok(mut admin) = PgConnection::connect(&with_db(&base_url(), "postgres")).await {
        let _ = admin
            .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
            .await;
    }
}

fn state(pg: PgPool) -> api::AppState {
    let kill_switch = Arc::new(risk::KillSwitch::new(false));
    let risk_gate = Arc::new(risk::RiskGate::new(
        risk::GlobalRiskLimits::default(),
        Arc::clone(&kill_switch),
    ));
    let paper_engine = Arc::new(execution::paper::PaperTradingEngine::realistic());
    let execution_engine = Arc::new(execution::ExecutionEngine::new(Arc::new(
        paper_engine.multi_asset_broker(),
    )));
    let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(Arc::new(
        demand_manager::DemandRegistry::new(Arc::new(demand_manager::NoopPipelineFactory)),
    )));
    let clickhouse_url = "http://127.0.0.1:9".to_string();
    let backtest = backtest::BacktestManager::new(clickhouse_url.clone(), pg.clone());
    let models = model_registry::ModelManager::from_env(pg.clone());
    let inference = model_registry::InferenceGateway::new(
        pg.clone(),
        Arc::new(model_registry::sidecar::SidecarClient::from_env()),
    );
    let agent = Arc::new(api::agent::AgentManager::new(
        pg.clone(),
        "http://127.0.0.1:9".into(),
        cfg::model::AgentConfig::default(),
    ));
    api::AppState::new(
        pg,
        risk_gate,
        kill_switch,
        execution_engine,
        paper_engine,
        gateway,
        backtest,
        models,
        inference,
        cfg::model::EmailConfig::default(),
        clickhouse_url,
        None,
        agent,
        None,
    )
}

async fn call(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if ctype.starts_with("text/event-stream") {
        // An open stream: the headers are the answer.
        return (status, Value::Null, ctype);
    }
    let bytes = axum::body::to_bytes(resp.into_body(), 16 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ctype,
    )
}

/// One digest over every table a session write could touch. A write to any of
/// them (a row added, a status flipped, an `updated_at` moved) changes it.
async fn fingerprint(pg: &PgPool) -> Vec<(String, String)> {
    let tables = [
        "research_sessions",
        "session_events",
        "agenda_items",
        "agenda_item_sponsors",
        "session_digests",
        "session_leases",
        "session_day_budget",
        "profile_standing",
        "profile_judgement",
        "agent_conversations",
        "jobs",
    ];
    let mut out = Vec::new();
    for t in tables {
        let q = format!(
            "SELECT COALESCE(md5(string_agg(x::text, '|' ORDER BY x::text)), '-') || ':' || count(*)::text FROM {t} x"
        );
        let r: Result<(String,), _> = sqlx::query_as(&q).fetch_one(pg).await;
        out.push((
            t.to_string(),
            r.map_or_else(|e| format!("absent: {e}"), |r| r.0),
        ));
    }
    out
}

fn charter_json() -> Value {
    use research_session::charter::{ExplorationShare, Objective, Scope};
    serde_json::to_value(research_session::Charter {
        scope: Scope::default(),
        objective: Objective::BuildKnowledge { about: vec![] },
        seeds: vec![],
        permitted_workflows: vec![],
        exclusions: vec![],
        standing_constraints: vec![],
        exploration_share: ExplorationShare::default(),
        stop_when: vec![],
        delta_practical_default: None,
        version: 1,
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_token_is_refused_by_every_session_write_and_may_read() {
    let Some((pg, name)) = scratch().await else {
        eprintln!(
            "SKIPPED: no Postgres that can create databases at {} (set SESSION_TEST_PG_URL)",
            base_url()
        );
        return;
    };

    // The owner of a session: one person, who is also who the agent works for.
    let user: (Uuid,) = sqlx::query_as("INSERT INTO users (email) VALUES ($1) RETURNING user_id")
        .bind(format!("{}@agent-post.test", Uuid::new_v4().simple()))
        .fetch_one(&pg)
        .await
        .unwrap();
    let user = user.0;

    // A human (web:full), minted as login mints it.
    let human = format!("h{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO sessions (token, user_id, kind, scopes, expires_at, last_seen_at) \
         VALUES ($1, $2, 'web', ARRAY['web:full'], now() + interval '1 day', now())",
    )
    .bind(&human)
    .bind(user)
    .execute(&pg)
    .await
    .unwrap();

    // Agent token 1: the project-bound research token, production minting path.
    let project = api::projects::ProjectStore::new(pg.clone())
        .create_research(user, "adversarial", None, &[], None, None)
        .await
        .expect("a research project");
    let minted = api::orchestrator::SessionOrchestrator::new(pg.clone())
        .mint_token(user, project.project_id)
        .await
        .expect("the research token");
    assert!(
        !api::auth::scopes::permits(&minted.scopes, "web:full"),
        "the fixture would be meaningless if the minted token held web:full"
    );

    // Agent token 2: the internal agent run's token, as AgentManager mints it.
    let run_id = Uuid::new_v4();
    let run_token = format!("r{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO sessions (token, user_id, label, kind, expires_at) \
         VALUES ($1, $2, $3, 'service', now() + interval '7 days')",
    )
    .bind(&run_token)
    .bind(user)
    .bind(format!("agent-run-{run_id}"))
    .execute(&pg)
    .await
    .unwrap();

    // A running session this user owns, with a proposed and a queued item.
    let convo = Uuid::new_v4();
    let session = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_conversations (conversation_id, user_id, provider, model) \
         VALUES ($1, $2, 'test-fixture', 'test-fixture')",
    )
    .bind(convo)
    .bind(user)
    .execute(&pg)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO research_sessions (session_id, conversation_id, user_id, tier, continuous, question) \
         VALUES ($1, $2, $3, 'test-fixture', false, 'what works?')",
    )
    .bind(session)
    .bind(convo)
    .bind(user)
    .execute(&pg)
    .await
    .unwrap();
    for (kind, st) in [("session_created", "draft"), ("session_started", "running")] {
        sqlx::query("INSERT INTO session_events (session_id, kind, state) VALUES ($1, $2, $3)")
            .bind(session)
            .bind(kind)
            .bind(st)
            .execute(&pg)
            .await
            .unwrap();
    }
    let proposed = Uuid::new_v4();
    let queued = Uuid::new_v4();
    for (id, st, q) in [
        (proposed, "proposed", "a proposed question"),
        (queued, "queued", "a queued one"),
    ] {
        sqlx::query(
            "INSERT INTO agenda_items (item_id, session_id, question, subject, workflow, dedupe_key, status) \
             VALUES ($1, $2, $3, 'BTC-USD', 'investigate', $4, $5)",
        )
        .bind(id)
        .bind(session)
        .bind(q)
        .bind(format!("key-{id}"))
        .bind(st)
        .execute(&pg)
        .await
        .unwrap();
    }

    // An item carrying one ref of every shape a source writes, and one real
    // memory entry for the first to resolve to (S-11.2: a ref opens in the desk).
    // (migration 0122 needs pgvector, which the scratch cluster lacks: then the entry
    // cannot exist and the first ref must say so rather than pretend.)
    let memory_up = api::memory_store::MemoryStore::new(pg.clone())
        .put(
            user,
            &harness::memory::Entry::finding(
                "f_ref1",
                "BTC-USD",
                "a finding the card opens",
                harness::memory::DataWindow::new("2026-01-01", "2026-02-01"),
                harness::memory::Provenance::new("run_x", "tool_x", "1", "hash_x"),
            ),
        )
        .await
        .is_ok();
    eprintln!("agent_memory available in this scratch database: {memory_up}");
    let reffed = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agenda_items (item_id, session_id, question, subject, workflow, dedupe_key, status, source_refs) \
         VALUES ($1, $2, 'a question with refs', 'BTC-USD', 'investigate', $3, 'queued', $4)",
    )
    .bind(reffed)
    .bind(session)
    .bind(format!("key-{reffed}"))
    .bind(json!([
        "f_ref1",
        "f_gone",
        format!("survey:{}", Uuid::new_v4()),
        "regime:BTC-USD:calm->stormy@2026-01-01",
        format!("monitor:{}", Uuid::new_v4()),
    ]))
    .execute(&pg)
    .await
    .unwrap();

    let app = api::routes::router(state(pg.clone()));
    let s = format!("/api/research/sessions/{session}");
    let item = |verb: &str| format!("{s}/agenda/{proposed}/{verb}");

    // Every mutating route research_sessions.rs serves. The body is valid for
    // each, so a 403 is the scope check and not an extractor's refusal.
    let writes: Vec<(&str, String, Option<Value>)> = vec![
        (
            "POST",
            "/api/research/sessions".into(),
            Some(json!({"question": "does it work?"})),
        ),
        ("POST", format!("{s}/pause"), None),
        ("POST", format!("{s}/resume"), None),
        ("POST", format!("{s}/stop"), None),
        (
            "POST",
            format!("{s}/agenda"),
            Some(json!({"question": "an agent's own item"})),
        ),
        (
            "POST",
            format!("{s}/agenda/order"),
            Some(json!({"order": [queued, proposed]})),
        ),
        ("POST", item("approve"), None),
        ("POST", item("veto"), None),
        ("POST", item("prioritise"), None),
        (
            "POST",
            item("redirect"),
            Some(json!({"question": "steered"})),
        ),
        ("PUT", format!("{s}/charter"), Some(charter_json())),
        (
            "PUT",
            format!("{s}/day-budget"),
            Some(json!({"trials_per_day": 9})),
        ),
        (
            "POST",
            "/api/research/profiles/some-profile/scorecard".into(),
            Some(json!({"health": "measured", "reason": "an agent grading itself"})),
        ),
    ];
    assert_eq!(writes.len(), 13);

    let before = fingerprint(&pg).await;
    for (who, token) in [
        ("project-bound research token", minted.token.as_str()),
        ("agent run token", run_token.as_str()),
    ] {
        for (method, path, body) in &writes {
            let (status, answer, _) = call(&app, method, path, token, body.clone()).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{who} must be refused by {method} {path}, got {status}: {answer}"
            );
            assert_eq!(
                answer["error"], "not_a_human_session",
                "{method} {path}: {answer}"
            );
        }
    }
    assert_eq!(
        before,
        fingerprint(&pg).await,
        "an agent token's refused writes must leave no row, event or status changed"
    );
    // The proposed item is exactly as it was.
    let st: (String,) = sqlx::query_as("SELECT status FROM agenda_items WHERE item_id = $1")
        .bind(proposed)
        .fetch_one(&pg)
        .await
        .unwrap();
    assert_eq!(st.0, "proposed");

    // A token that is not a session at all gets 401, not 403.
    let (status, _, _) = call(&app, "POST", &format!("{s}/pause"), "no-such-token", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The same agent tokens read: the session, its board, its digests, its
    // stream, the contradiction pairs and a profile's standing.
    let reads = [
        "/api/research/sessions".to_string(),
        s.clone(),
        format!("{s}/agenda"),
        format!("{s}/digests"),
        format!("{s}/events"),
        format!("{s}/agenda/{reffed}/refs"),
        format!("{s}/day-budget"),
        "/api/research/contradictions".to_string(),
        "/api/research/profiles/some-profile/standing".to_string(),
    ];
    for token in [minted.token.as_str(), run_token.as_str()] {
        for path in &reads {
            let (status, answer, ctype) = call(&app, "GET", path, token, None).await;
            assert_eq!(status, StatusCode::OK, "GET {path}: {answer}");
            if path.ends_with("/events") {
                assert!(ctype.starts_with("text/event-stream"), "{ctype}");
            }
        }
        // The run-keyed reads answer a caller that is not a run in a session
        // with a real answer, never a refusal of the credential.
        for path in [
            "/api/research/sessions/current",
            "/api/research/sessions/current/stage",
        ] {
            let (status, _, _) = call(&app, "GET", path, token, None).await;
            assert_ne!(status, StatusCode::FORBIDDEN, "GET {path}");
            assert_ne!(status, StatusCode::UNAUTHORIZED, "GET {path}");
        }
    }

    // What each ref opens.
    let (status, body, _) = call(
        &app,
        "GET",
        &format!("{s}/agenda/{reffed}/refs"),
        &minted.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let refs = body["refs"].as_array().expect("refs");
    assert_eq!(refs.len(), 5, "{body}");
    if memory_up {
        assert_eq!(refs[0]["found"], true);
        assert_eq!(refs[0]["kind"], "memory_entry");
        assert_eq!(refs[0]["open"]["route"], "/api/memory/findings");
        assert_eq!(refs[0]["open"]["id"], "f_ref1");
    } else {
        assert_eq!(refs[0]["found"], false, "{body}");
    }
    assert_eq!(
        refs[1]["found"], false,
        "an entry that is not there says so: {body}"
    );
    assert_eq!(refs[2]["kind"], "survey_run");
    assert_eq!(refs[2]["found"], false);
    assert_eq!(refs[3]["kind"], "regime_flip");
    assert_eq!(refs[3]["found"], true);
    assert_eq!(refs[4]["kind"], "monitor_event");
    assert_eq!(refs[4]["found"], false);
    // The day budget route counts spend the way the driver's day gate does:
    // finished runs' trials today, an unmeasured run beside the sum and not in
    // it as a zero, a live child named because its spend is not in yet.
    for (spent, finished) in [
        (Some(3), true),
        (Some(4), true),
        (None, true),
        (None, false),
    ] {
        sqlx::query(
            "INSERT INTO agent_runs (run_id, user_id, goal, status, provider, model, session_id, trials_spent, finished_at) \
             VALUES ($1, $2, 'g', $3, 'test-fixture', 'test-fixture', $4, $5, CASE WHEN $6 THEN now() END)",
        )
        .bind(Uuid::new_v4())
        .bind(user)
        .bind(if finished { "completed" } else { "running" })
        .bind(session)
        .bind(spent)
        .bind(finished)
        .execute(&pg)
        .await
        .expect("a run row");
    }
    let (status, day, _) = call(&app, "GET", &format!("{s}/day-budget"), &minted.token, None).await;
    assert_eq!(status, StatusCode::OK, "{day}");
    assert_eq!(day["unit"], "trials");
    assert_eq!(day["spent_today"]["trials"], 7);
    assert_eq!(day["spent_today"]["runs_finished"], 3);
    assert_eq!(
        day["spent_today"]["unmeasured_runs"], 1,
        "not recorded is not zero: {day}"
    );
    assert_eq!(day["live_children"], 1);
    assert_eq!(day["is_floor"], true);
    assert!(day["usd_spent"].is_null(), "no dollars are recorded: {day}");
    assert_eq!(day["day_zone"], "UTC");
    // Another person's session is not readable.
    let stranger: (Uuid,) =
        sqlx::query_as("INSERT INTO users (email) VALUES ($1) RETURNING user_id")
            .bind(format!("{}@agent-post.test", Uuid::new_v4().simple()))
            .fetch_one(&pg)
            .await
            .unwrap();
    let other = format!("o{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO sessions (token, user_id, kind, scopes, expires_at, last_seen_at) \
         VALUES ($1, $2, 'web', ARRAY['web:full'], now() + interval '1 day', now())",
    )
    .bind(&other)
    .bind(stranger.0)
    .execute(&pg)
    .await
    .unwrap();
    let (status, _, _) = call(
        &app,
        "GET",
        &format!("{s}/agenda/{reffed}/refs"),
        &other,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = call(&app, "GET", &format!("{s}/day-budget"), &other, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The control: a human sent the same requests is not refused. Without this
    // a 403 above could be a route that refuses everyone.
    for (method, path, body) in &writes {
        let (status, answer, _) = call(&app, method, path, &human, body.clone()).await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "a human must pass {method} {path}: {answer}"
        );
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert!(
            answer["error"] != "not_a_human_session",
            "{method} {path}: {answer}"
        );
    }

    drop_db(pg, &name).await;
}
