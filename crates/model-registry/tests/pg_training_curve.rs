//! The Studio's two reads against PostgreSQL (migrations 0200, 0201; Set H
//! H-4.5, H-5.5): which strategies use a model, and a training run's loss curve.
//!
//! ```text
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/<scratch> \
//!   cargo test -j 2 -p model-registry --test pg_training_curve -- --ignored --test-threads=1
//! ```

use model_registry::manager::ModelManager;
use serde_json::json;
use uuid::Uuid;

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects")
}

async fn model(pg: &sqlx::PgPool, slug: &str) -> (String, Uuid) {
    let user = Uuid::new_v4();
    let model_id = format!("m-{}", Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, definition_json, created_by)
         VALUES ($1, $2, 'm', 'forecaster', 'crypto', '{}'::jsonb, $3)",
    )
    .bind(&model_id)
    .bind(slug)
    .bind(user)
    .execute(pg)
    .await
    .unwrap();
    (model_id, user)
}

/// A strategy whose nodes are `nodes`, with the revision row its head must point at.
async fn strategy(pg: &sqlx::PgPool, nodes: serde_json::Value) -> String {
    let sid = format!("s-{}", Uuid::new_v4().simple());
    let rev = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO strategy_revision (revision_hash, strategy_id, parent_hash, definition_version,
             asset_class, definition_json, definition_bytes, actor_kind, actor_id, seq, prev_hash, row_hash)
         VALUES ($1,$2,NULL,'1.1','crypto_spot_cex','{}'::jsonb,'\\x00'::bytea,'agent','t',0,'\\x'::bytea,'\\x'::bytea)",
    )
    .bind(&rev)
    .bind(&sid)
    .execute(pg)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO strategy_definitions (strategy_id, definition_version, asset_class, definition_json, head_revision)
         VALUES ($1,'1.1','crypto_spot_cex',$2,$3)",
    )
    .bind(&sid)
    .bind(json!({ "nodes": nodes }))
    .bind(&rev)
    .execute(pg)
    .await
    .unwrap();
    sid
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0201"]
async fn a_strategy_that_names_the_model_by_id_or_slug_is_used_by_it() {
    let pg = pool().await;
    let slug = format!("slug-{}", Uuid::new_v4().simple());
    let (mid, _) = model(&pg, &slug).await;
    let (other, _) = model(&pg, &format!("other-{}", Uuid::new_v4().simple())).await;
    let by_id = strategy(&pg, json!([{"id":"m1","type":"model_forecast","model_ref": mid, "direction":"bullish","min_confidence":0.6}])).await;
    let by_slug = strategy(&pg, json!([{"id":"x","type":"signal"},{"id":"m1","type":"model_forecast","model_ref": slug, "direction":"any","min_confidence":0.5}])).await;
    let unrelated = strategy(&pg, json!([{"id":"m1","type":"model_forecast","model_ref": other, "direction":"any","min_confidence":0.5}])).await;

    let mgr = ModelManager::from_env(pg.clone());
    let used = mgr.get_used_by(&mid).await.expect("the read does not fail");
    let ids: Vec<&str> = used
        .iter()
        .filter_map(|u| u["strategy_id"].as_str())
        .collect();
    assert!(ids.contains(&by_id.as_str()), "by id: {ids:?}");
    assert!(ids.contains(&by_slug.as_str()), "by slug: {ids:?}");
    assert!(
        !ids.contains(&unrelated.as_str()),
        "another model's strategy leaked: {ids:?}"
    );

    // Removing the reference removes it.
    sqlx::query("UPDATE strategy_definitions SET definition_json = '{\"nodes\": []}'::jsonb WHERE strategy_id = $1")
        .bind(&by_id)
        .execute(&pg)
        .await
        .unwrap();
    let used = mgr.get_used_by(&mid).await.unwrap();
    assert!(!used.iter().any(|u| u["strategy_id"] == by_id.as_str()));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0201"]
async fn progress_frames_leave_a_curve_the_owner_can_read() {
    let pg = pool().await;
    let (mid, user) = model(&pg, &format!("c-{}", Uuid::new_v4().simple())).await;
    let run = Uuid::new_v4();
    sqlx::query("INSERT INTO training_runs (run_id, model_id, status, phase, created_by) VALUES ($1,$2,'running','fitting',$3)")
        .bind(run)
        .bind(&mid)
        .bind(user)
        .execute(&pg)
        .await
        .unwrap();
    let mgr = ModelManager::from_env(pg.clone());
    for (e, t, v) in [(1, 0.9, 1.0), (2, 0.6, 0.8), (3, 0.4, 0.7)] {
        mgr.apply_nats_progress(
            run,
            "fitting",
            30.0 * f32::from(e as u8),
            Some(json!({"epoch": e, "train_loss": t, "val_loss": v})),
        )
        .await;
    }
    // A re-reported epoch keeps the first row; a frame with no epoch stores nothing.
    mgr.apply_nats_progress(
        run,
        "fitting",
        90.0,
        Some(json!({"epoch": 3, "train_loss": 9.0, "val_loss": 9.0})),
    )
    .await;
    mgr.apply_nats_progress(
        run,
        "evaluating",
        95.0,
        Some(json!({"final_train_loss": 0.4})),
    )
    .await;
    // No validation split: NaN arrives as null and is stored as NULL, not 0.
    mgr.apply_nats_progress(
        run,
        "fitting",
        99.0,
        Some(json!({"epoch": 4, "train_loss": 0.3, "val_loss": null})),
    )
    .await;

    let curve = mgr.training_curve(&mid, run, user).await.unwrap();
    assert_eq!(curve.len(), 4);
    assert_eq!(
        curve[2].train_loss,
        Some(0.4),
        "the first report of epoch 3 stands"
    );
    assert_eq!(curve[3].val_loss, None);

    // Someone else's model's curve is not served.
    assert!(mgr.training_curve(&mid, run, Uuid::new_v4()).await.is_err());
    // Nor is a run under another model.
    let (other, ouser) = model(&pg, &format!("o-{}", Uuid::new_v4().simple())).await;
    assert!(mgr.training_curve(&other, run, ouser).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs DATABASE_URL with migrations through 0201"]
async fn the_platform_role_can_append_metrics_and_never_edit_them() {
    let pg = pool().await;
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(sqlx::query(
        "INSERT INTO mlops.training_metric (run_id, epoch, train_loss) VALUES ($1, 1, 0.5)"
    )
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .is_ok());
    assert!(sqlx::query("SELECT 1 FROM mlops.training_metric")
        .fetch_all(&mut *tx)
        .await
        .is_ok());
    assert!(sqlx::query("UPDATE mlops.training_metric SET epoch = 2")
        .execute(&mut *tx)
        .await
        .is_err());
    tx.rollback().await.unwrap();
    let mut tx = pg.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(sqlx::query("DELETE FROM mlops.training_metric")
        .execute(&mut *tx)
        .await
        .is_err());
    tx.rollback().await.unwrap();
}
