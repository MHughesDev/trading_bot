//! Set U U-3.4's HTTP criterion: a v3 backtest submitted over HTTP runs and
//! reports.
//!
//! The platform's own router and `AppState`, built in process the way
//! `apps/platform/src/main.rs` builds them — but nothing is started: no
//! collector, no pipeline, no live poll. The request goes through the real
//! extractor, the real handler and the backtest manager's job, which reads its
//! bars from ClickHouse and runs the v3 graph on the strategy engine; the tile
//! is read back over HTTP.
//!
//! It needs a Postgres that can create databases (a scratch database is made,
//! every migration applied, and dropped) and a ClickHouse database holding
//! stored bars — a scratch copy, never the platform's own:
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//! BACKTEST_CLICKHOUSE_URL=http://…@localhost:8123/u55_scratch \
//! BACKTEST_INSTRUMENT=BTC-USD BACKTEST_FROM=2026-09-01T00:00:00Z BACKTEST_TO=2026-09-11T00:00:00Z \
//!   cargo test -j 2 -p api --test v3_backtest_over_http -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name}"))
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// A fresh database with every migration that applies here (one needing an
/// extension this cluster lacks is skipped and reported, as `deployment_path`
/// does).
async fn scratch() -> (PgPool, String) {
    let name = format!("v3_http_{}", Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&with_db(&env("DATABASE_URL"), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let pg = PgPool::connect(&with_db(&env("DATABASE_URL"), &name))
        .await
        .unwrap();
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
    (pg, name)
}

async fn drop_db(pg: PgPool, name: &str) {
    pg.close().await;
    let mut admin = PgConnection::connect(&with_db(&env("DATABASE_URL"), "postgres"))
        .await
        .unwrap();
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

/// The platform's state, built as `main.rs` builds it, with nothing spawned.
fn state(pg: PgPool, clickhouse_url: String) -> api::AppState {
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
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a Postgres that can create databases and BACKTEST_CLICKHOUSE_URL with stored bars"]
async fn a_v3_backtest_submitted_over_http_runs_and_reports() {
    let (pg, name) = scratch().await;
    let instrument = env("BACKTEST_INSTRUMENT");

    // Someone signed in.
    let user: (Uuid,) = sqlx::query_as("INSERT INTO users (email) VALUES ($1) RETURNING user_id")
        .bind(format!("{}@v3-http.test", Uuid::new_v4().simple()))
        .fetch_one(&pg)
        .await
        .unwrap();
    let token = format!("tok_{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO sessions (token, user_id) VALUES ($1, $2)")
        .bind(&token)
        .bind(user.0)
        .execute(&pg)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO instruments (instrument_id, asset_class, venue_id, base_precision, \
           quote_precision, tick_size, lot_size, trading_hours_json, halt_policy, trust_tier) \
         VALUES ($1, 'crypto_spot_cex', 'coinbase', 8, 2, 0.01, 0.0001, '{}', 'non_haltable', \
           'centralized_exchange') ON CONFLICT (instrument_id) DO NOTHING",
    )
    .bind(&instrument)
    .execute(&pg)
    .await
    .unwrap();

    let app = api::routes::router(state(pg.clone(), env("BACKTEST_CLICKHOUSE_URL")));

    let graph = serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "http_ema_cross",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"f",    "op":"ema",           "args":{"x":"close", "n":5}},
        {"id":"s",    "op":"ema",           "args":{"x":"close", "n":20}},
        {"id":"up",   "op":"crossed_above", "args":{"a":"f", "b":"s"}},
        {"id":"entry","op":"and",           "args":{"a":"up", "b":"state.flat"}},
        {"id":"down", "op":"crossed_below", "args":{"a":"f", "b":"s"}},
        {"id":"leave","op":"and",           "args":{"a":"down", "b":"state.long"}}
      ],
      "decide": {"rules": [
        {"when":"entry","then":{"kind":"target","size":{"mode":"units","x":0.01}}},
        {"when":"leave","then":{"kind":"close"}}
      ]}
    });
    let request = |graph: serde_json::Value, timeframe: &str| {
        serde_json::json!({
          "graph": graph,
          "instrument_id": instrument,
          "asset_class": "crypto_spot_cex",
          "timeframe": timeframe,
          "start": env("BACKTEST_FROM"),
          "end": env("BACKTEST_TO"),
          "initial_balance": "10000",
          "auto_collect": false,
        })
    };

    // A graph that does not compile, and a timeframe that is not the graph's,
    // are refused at the door.
    let mut broken = graph.clone();
    broken["nodes"][0]["op"] = serde_json::json!("no_such_op");
    let (status, body) = call(
        &app,
        "POST",
        "/api/backtests",
        &token,
        Some(request(broken, "1h")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let (status, body) = call(
        &app,
        "POST",
        "/api/backtests",
        &token,
        Some(request(graph.clone(), "1d")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "timeframe_mismatch");
    // Nobody signed in: refused.
    let (status, _) = call(
        &app,
        "POST",
        "/api/backtests",
        "nobody",
        Some(request(graph.clone(), "1h")),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = call(
        &app,
        "POST",
        "/api/backtests",
        &token,
        Some(request(graph, "1h")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["engine"], "strategy-engine/v3");
    let id = body["id"].as_str().expect("an id").to_string();

    // Poll the tile until the job settles.
    let mut tile = serde_json::Value::Null;
    for _ in 0..240 {
        let (status, body) = call(&app, "GET", &format!("/api/backtests/{id}"), &token, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        tile = body;
        if matches!(
            tile["status"].as_str(),
            Some("completed" | "failed" | "cancelled")
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!(
        "tile: status {} progress {} error {}",
        tile["status"], tile["progress"], tile["error"]
    );
    assert_eq!(tile["status"], "completed", "{tile:#}");
    let result = &tile["result"];
    println!(
        "result keys: {:?}",
        result
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>())
    );
    assert!(
        result.is_object(),
        "the tile carries the run's result: {tile:#}"
    );
    println!(
        "engine {} status {} orders {} trades {} decisions {}",
        result["engine"],
        result["status"],
        result["total_orders"],
        result["trades"].as_array().map_or(0, Vec::len),
        result["manifest"]["decisions"]
    );
    assert!(
        result["manifest"]["decisions"].as_u64().unwrap_or(0) > 0,
        "the engine decided on the stored bars"
    );
    assert!(
        result["total_orders"].as_u64().unwrap_or(0) > 0,
        "and the EMA cross traded"
    );

    // Another person cannot read it.
    let other: (Uuid,) = sqlx::query_as("INSERT INTO users (email) VALUES ($1) RETURNING user_id")
        .bind(format!("{}@v3-http.test", Uuid::new_v4().simple()))
        .fetch_one(&pg)
        .await
        .unwrap();
    let theirs = format!("tok_{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO sessions (token, user_id) VALUES ($1, $2)")
        .bind(&theirs)
        .bind(other.0)
        .execute(&pg)
        .await
        .unwrap();
    let (status, _) = call(&app, "GET", &format!("/api/backtests/{id}"), &theirs, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    drop_db(pg, &name).await;
}
