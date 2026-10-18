//! N-4.5 (Set H 4.5 to 4.8) against PostgreSQL: used-by, traces and rollups, the inference rate
//! limit and the nightly retrain's due list. Each test fails if the thing it covers is removed.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -p model-registry --test pg_model_ops -- --ignored --test-threads=1
//! ```
//! The scratch database needs migrations 0001-0004, 0017-0020, 0028 and 0222.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use model_registry::scheduler::RetrainScheduler;
use model_registry::sidecar::SidecarClient;
use model_registry::{InferenceGateway, ModelManager};
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

async fn model(pg: &sqlx::PgPool, definition: serde_json::Value, status: &str) -> String {
    let id = format!("m-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, status, created_by)
         VALUES ($1, $1, 'm', 'forecaster', 'crypto', $2, $3, $4)",
    )
    .bind(&id)
    .bind(definition)
    .bind(status)
    .bind(Uuid::new_v4())
    .execute(pg)
    .await
    .unwrap();
    id
}

fn gateway(pg: &sqlx::PgPool, max: usize) -> Arc<InferenceGateway> {
    InferenceGateway::with_rate_limit(
        pg.clone(),
        Arc::new(SidecarClient::from_env()),
        max,
        Duration::from_secs(60),
    )
}

// ---- H-4.5 used-by ---------------------------------------------------------

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn used_by_lists_the_strategies_that_reference_the_model_and_only_those() {
    let pg = pool().await;
    let m = model(&pg, serde_json::json!({}), "draft").await;
    let other = model(&pg, serde_json::json!({}), "draft").await;
    let sid = format!("s-{}", Uuid::new_v4().simple());
    let other_sid = format!("s-{}", Uuid::new_v4().simple());
    for (id, referenced) in [(&sid, &m), (&other_sid, &other)] {
        // 0140: a definition names the revision it serves, so the revision goes in first.
        let definition = serde_json::json!({ "name": id, "nodes": [{ "model_ref": referenced }] });
        let revision = id.trim_start_matches("s-").repeat(2);
        sqlx::query(
            "INSERT INTO strategy_revision (revision_hash, strategy_id, definition_version, asset_class, \
               definition_json, definition_bytes, actor_kind, actor_id, seq, prev_hash, row_hash) \
             VALUES ($1, $2, '1.0', 'crypto', $3, $4, 'human', 'test', 0, ''::bytea, ''::bytea)",
        )
        .bind(&revision)
        .bind(id)
        .bind(&definition)
        .bind(definition.to_string().into_bytes())
        .execute(&pg)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO strategy_definitions (strategy_id, asset_class, definition_json, head_revision)
             VALUES ($1, 'crypto', $2, $3)",
        )
        .bind(id)
        .bind(&definition)
        .bind(&revision)
        .execute(&pg)
        .await
        .unwrap();
    }
    let manager = ModelManager::new(
        pg.clone(),
        Arc::new(SidecarClient::from_env()),
        Arc::new(model_registry::datasets::DatasetManager::new(pg.clone())),
    );
    let used = manager.get_used_by(&m).await.unwrap();
    let ids: Vec<&str> = used
        .iter()
        .filter_map(|u| u["strategy_id"].as_str())
        .collect();
    assert_eq!(
        ids,
        vec![sid.as_str()],
        "only the strategy that references this model"
    );
    assert!(manager
        .get_used_by("no-such-model")
        .await
        .unwrap()
        .is_empty());
}

// ---- H-4.6 traces and rollups ----------------------------------------------

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn the_trace_rollup_counts_outcomes_and_reports_latency() {
    let pg = pool().await;
    let m = model(&pg, serde_json::json!({}), "draft").await;
    for (result, ms) in [
        ("hit", 10),
        ("hit", 30),
        ("error", 100),
        ("rate_limited", 0),
    ] {
        sqlx::query(
            "INSERT INTO model_events (model_id, kind, payload, actor) VALUES ($1, 'inference_trace', $2, $3)",
        )
        .bind(&m)
        .bind(serde_json::json!({ "result": result, "latency_ms": ms }))
        .bind(Uuid::nil())
        .execute(&pg)
        .await
        .unwrap();
    }
    let rollup = gateway(&pg, 10).trace_rollup(&m).await.unwrap();
    let hit = rollup
        .iter()
        .find(|r| r["result"] == "hit")
        .expect("hit row");
    assert_eq!(hit["count"], 2);
    assert_eq!(hit["latency_ms_p50"], 20.0);
    assert!(rollup
        .iter()
        .any(|r| r["result"] == "rate_limited" && r["count"] == 1));
    assert_eq!(
        rollup[0]["result"], "hit",
        "ordered by count, largest first"
    );
}

// ---- H-4.7 rate limit ------------------------------------------------------

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn a_model_past_its_rate_limit_is_refused_and_the_refusal_is_traced() {
    let pg = pool().await;
    let m = model(&pg, serde_json::json!({}), "draft").await;
    sqlx::query("INSERT INTO model_aliases (model_id, alias, version, updated_by) VALUES ($1, 'production', 1, $2)")
        .bind(&m)
        .bind(Uuid::new_v4())
        .execute(&pg)
        .await
        .unwrap();
    let gw = gateway(&pg, 2);
    // Distinct features so the prediction cache cannot answer. There is no artifact, so each
    // admitted call abstains; the third is refused by the limit before it gets that far.
    for i in 0..3 {
        let f = HashMap::from([("x".to_string(), f64::from(i))]);
        assert!(gw.forecast(&m, "production", "BTC-USD", &f).await.is_none());
    }
    let rollup = gw.trace_rollup(&m).await.unwrap();
    let count = |what: &str| {
        rollup
            .iter()
            .find(|r| r["result"] == what)
            .map_or(0, |r| r["count"].as_i64().unwrap())
    };
    assert_eq!(count("abstain"), 2, "{rollup:?}");
    assert_eq!(count("rate_limited"), 1, "{rollup:?}");
    // Another model is unaffected by this one's window.
    assert!(gw.admit("some-other-model"));
}

// ---- H-4.8 nightly retrain -------------------------------------------------

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222"]
async fn only_an_auto_retrain_model_with_no_run_in_flight_is_due() {
    let pg = pool().await;
    let due = model(&pg, serde_json::json!({ "auto_retrain": true }), "draft").await;
    let off = model(&pg, serde_json::json!({ "auto_retrain": false }), "draft").await;
    let archived = model(&pg, serde_json::json!({ "auto_retrain": true }), "archived").await;
    let running = model(&pg, serde_json::json!({ "auto_retrain": true }), "draft").await;
    sqlx::query("INSERT INTO training_runs (run_id, model_id, created_by, status) VALUES ($1, $2, $3, 'running')")
        .bind(Uuid::new_v4())
        .bind(&running)
        .bind(Uuid::new_v4())
        .execute(&pg)
        .await
        .unwrap();
    let ids: Vec<String> = RetrainScheduler::due_models(&pg)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.0)
        .collect();
    assert!(ids.contains(&due));
    for not_due in [&off, &archived, &running] {
        assert!(!ids.contains(not_due), "{not_due} must not be due");
    }
}

// ---- N-4.1 serving consumes the calibrated spread ---------------------------

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0222 and 0209"]
async fn the_calibrated_spread_comes_from_the_newest_wrapper_of_that_version() {
    let pg = pool().await;
    let m = model(&pg, serde_json::json!({}), "draft").await;
    let wrapper = |version: i32, wraps: i32, spread: serde_json::Value| {
        let pg = pg.clone();
        let m = m.clone();
        async move {
            sqlx::query(
                "INSERT INTO model_versions (model_id, version, status, config_json, created_by, family)
                 VALUES ($1, $2, 'candidate', $3, $4, 'calibration')",
            )
            .bind(&m)
            .bind(version)
            .bind(serde_json::json!({ "wraps": { "model_id": m, "version": wraps },
                                       "calibration": { "spread_90": spread } }))
            .bind(Uuid::new_v4())
            .execute(&pg)
            .await
            .unwrap();
        }
    };
    wrapper(2, 1, serde_json::json!([null, 0.01, 0.02])).await;
    wrapper(3, 1, serde_json::json!([null, null, 0.05, null])).await;
    wrapper(4, 7, serde_json::json!([0.9])).await; // wraps a different version
    let gw = gateway(&pg, 10);
    assert_eq!(
        gw.calibrated_spread(&m, 1).await,
        Some(0.05),
        "newest wrapper, latest finite band"
    );
    assert_eq!(gw.calibrated_spread(&m, 7).await, Some(0.9));
    assert_eq!(
        gw.calibrated_spread(&m, 99).await,
        None,
        "a version nobody calibrated has none"
    );
}
