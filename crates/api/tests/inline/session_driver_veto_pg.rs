//! S-11.13 / S-11.5 / S-11.12: **a veto prevents its claim and shows in the
//! digest**, proven end to end against Postgres as `app_role`.
//!
//! The pieces each had a unit test (`claimable` over an in-memory log,
//! `pause_line` over a hand-built event). Nothing ran them in the order a person
//! does: click *veto* in the desk, let the driver iterate, read the digest. This
//! does, with the platform's own router for the click and the driver's own
//! `claimable` and `force_digest` for the rest.
//!
//! * the click is `POST /api/research/sessions/{id}/agenda/{item}/veto` through
//!   the real router with a `web:full` token;
//! * the veto's authority is the **log**: the item's row is flipped back to
//!   `queued` by the owner afterwards (a stale projection, a bad writer) and
//!   `claimable` still refuses it;
//! * the digest the driver writes lists the veto, in the *Paused* section, with
//!   the item's id, and the next-five list does not offer the vetoed item.
//!
//! Needs a Postgres that can create databases: default
//! `postgres://postgres:postgres@127.0.0.1:5433/postgres`, or
//! `SESSION_TEST_PG_URL`. A scratch database is made, migrated and dropped; with
//! no server the test prints `SKIPPED` and passes.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use research_session::agenda::ItemStatus;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, PgConnection, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

use super::*;

fn base_url() -> String {
    std::env::var("SESSION_TEST_PG_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/postgres".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// A migrated scratch database, as the owner.
async fn scratch() -> Option<(PgPool, String)> {
    let name = format!("veto_pg_{}", Uuid::new_v4().simple());
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
    let pg = PgPool::connect(&with_db(&base_url(), &name)).await.expect("scratch connect");
    // 0044 revokes on sqlx's bookkeeping table, so it needs one; without it 0044 fails and is
    // rolled back below, and every table made after it arrives with no grant to `app_role`.
    pg.execute("CREATE TABLE IF NOT EXISTS public._sqlx_migrations (version BIGINT PRIMARY KEY)")
        .await
        .unwrap();
    for mig in sqlx::migrate!("../../migrations").iter() {
        if mig.migration_type.is_down_migration() {
            continue;
        }
        let mut tx = pg.begin().await.unwrap();
        match tx.execute(&*mig.sql).await {
            Ok(_) => tx.commit().await.unwrap(),
            Err(_) => {
                // One needing an extension this cluster lacks (pgvector).
                tx.rollback().await.ok();
            }
        }
    }
    Some((pg, name))
}

async fn drop_db(pgs: &[&PgPool], name: &str) {
    for p in pgs {
        p.close().await;
    }
    if let Ok(mut admin) = PgConnection::connect(&with_db(&base_url(), "postgres")).await {
        let _ = admin
            .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
            .await;
    }
}

fn state(pg: PgPool) -> crate::AppState {
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
    let agent = Arc::new(crate::agent::AgentManager::new(
        pg.clone(),
        "http://127.0.0.1:9".into(),
        cfg::model::AgentConfig::default(),
    ));
    crate::AppState::new(
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_vetoed_item_is_not_claimed_and_the_digest_says_it_was_vetoed() {
    let Some((owner, name)) = scratch().await else {
        eprintln!(
            "SKIPPED: no Postgres that can create databases at {} (set SESSION_TEST_PG_URL)",
            base_url()
        );
        return;
    };
    // The platform's pool: every connection is app_role, so a missing GRANT on
    // anything the click, the claim check or the digest touches fails here.
    let app = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|c, _| {
            Box::pin(async move {
                c.execute("SET ROLE app_role").await?;
                Ok(())
            })
        })
        .connect(&with_db(&base_url(), &name))
        .await
        .expect("the platform pool connects");

    let user: (Uuid,) = sqlx::query_as("INSERT INTO users (email) VALUES ($1) RETURNING user_id")
        .bind(format!("{}@veto.test", Uuid::new_v4().simple()))
        .fetch_one(&owner)
        .await
        .unwrap();
    let user = user.0;
    let human = format!("h{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO sessions (token, user_id, kind, scopes, expires_at, last_seen_at) \
         VALUES ($1, $2, 'web', ARRAY['web:full'], now() + interval '1 day', now())",
    )
    .bind(&human)
    .bind(user)
    .execute(&owner)
    .await
    .unwrap();

    let convo = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_conversations (conversation_id, user_id, provider, model) \
         VALUES ($1, $2, 'test-fixture', 'test-fixture')",
    )
    .bind(convo)
    .bind(user)
    .execute(&owner)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO research_sessions (session_id, conversation_id, user_id, tier, continuous, question) \
         VALUES ($1, $2, $3, 'test-fixture', false, 'what works?')",
    )
    .bind(session_id)
    .bind(convo)
    .bind(user)
    .execute(&owner)
    .await
    .unwrap();
    for (kind, st) in [("session_created", "draft"), ("session_started", "running")] {
        sqlx::query("INSERT INTO session_events (session_id, kind, state) VALUES ($1, $2, $3)")
            .bind(session_id)
            .bind(kind)
            .bind(st)
            .execute(&owner)
            .await
            .unwrap();
    }
    let vetoed = Uuid::new_v4();
    let kept = Uuid::new_v4();
    for (id, q) in [(vetoed, "the question to veto"), (kept, "the question to keep")] {
        sqlx::query(
            "INSERT INTO agenda_items (item_id, session_id, question, subject, workflow, dedupe_key, status) \
             VALUES ($1, $2, $3, 'BTC-USD', 'investigate', $4, 'queued')",
        )
        .bind(id)
        .bind(session_id)
        .bind(q)
        .bind(format!("key-{id}"))
        .execute(&owner)
        .await
        .unwrap();
    }

    // The click: the real router, a human token.
    let router = crate::routes::router(state(app.clone()));
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/research/sessions/{session_id}/agenda/{vetoed}/veto"
                ))
                .header("authorization", format!("Bearer {human}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let said = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "the veto is accepted: {}",
        String::from_utf8_lossy(&said)
    );

    let worker = SessionWorker::new(
        app.clone(),
        Arc::new(crate::agent::AgentManager::new(
            app.clone(),
            "http://127.0.0.1:9".into(),
            cfg::model::AgentConfig::default(),
        )),
        crate::credentials::store::LlmCredentialStore::new(app.clone(), None),
        Arc::new(harness::ProfileSet::default()),
    );

    // The claim: only the item nobody vetoed is offered to the driver.
    let agenda = sessions::agenda(&app, session_id).await.unwrap();
    let folded = sessions::state(&app, session_id).await.unwrap();
    assert!(folded.vetoed.contains(&vetoed), "the log holds the veto");
    let live: Vec<Uuid> = SessionWorker::claimable(&agenda, &folded)
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(live, vec![kept], "a vetoed item is never offered to a claim");

    // The log is the authority, not the row: put the row back to `queued`, as a
    // stale projection or a second writer would, and the driver still refuses.
    sqlx::query("UPDATE agenda_items SET status = 'queued' WHERE item_id = $1")
        .bind(vetoed)
        .execute(&owner)
        .await
        .unwrap();
    let agenda = sessions::agenda(&app, session_id).await.unwrap();
    assert!(
        agenda
            .iter()
            .any(|i| i.id == vetoed && matches!(i.status, ItemStatus::Queued)),
        "the fixture really did resurrect the row"
    );
    let live: Vec<Uuid> = SessionWorker::claimable(&agenda, &folded)
        .iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(live, vec![kept], "the veto survives a resurrected row");

    // The digest: written by the driver, listing the veto.
    let session = sessions::get(&app, session_id)
        .await
        .unwrap()
        .expect("the session");
    assert!(
        worker
            .force_digest(&session, user, &agenda)
            .await
            .expect("a digest"),
        "a digest is written for a period that has a veto in it"
    );
    let rows: Vec<(Value,)> = sqlx::query_as("SELECT body FROM session_digests WHERE session_id = $1")
        .bind(session_id)
        .fetch_all(&app)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let body = rows[0].0.to_string();
    assert!(
        body.contains(&format!("you vetoed item {vetoed}; it will not be claimed")),
        "the digest names the veto and the item: {body}"
    );
    let digest: research_session::digest::Digest =
        serde_json::from_value(rows[0].0.clone()).expect("the stored body is a Digest");
    assert!(
        digest.unresolvable().is_empty(),
        "every line of it resolves: {:?}",
        digest.unresolvable()
    );
    let paused = digest
        .section(research_session::digest::Heading::Paused)
        .expect("a veto is in the Paused section");
    assert!(paused.lines.iter().any(|l| l.text.contains("vetoed")));
    // S-11.12's unit trap: the budget line beside a dollar bar says what it is
    // counted in, in the sentence and on the row.
    let unit: (String,) = sqlx::query_as("SELECT spend_unit FROM session_digests WHERE session_id = $1")
        .bind(session_id)
        .fetch_one(&app)
        .await
        .unwrap();
    assert_eq!(unit.0, "trials");
    let budget = digest
        .section(research_session::digest::Heading::Budget)
        .expect("a budget line");
    assert!(
        budget.lines.iter().all(|l| l.text.contains("trials")),
        "no number without its unit: {budget:?}"
    );
    // The next-five list is what *will* run. The row was resurrected to `queued`
    // above, so only the log says this item is vetoed; it must not be offered.
    let next = digest
        .section(research_session::digest::Heading::NextUp)
        .expect("the kept item is next up");
    assert!(
        next.lines.iter().any(|l| l.text.contains("the question to keep")),
        "{next:?}"
    );
    assert!(
        next.lines.iter().all(|l| !l.text.contains("the question to veto")),
        "the vetoed question is not offered as upcoming: {next:?}"
    );

    drop_db(&[&app, &owner], &name).await;
}
