//! Set U U-5.8 against PostgreSQL: a version's fold models are recorded once
//! each and read back into the registry snapshot, earliest training end first,
//! beside the version's own artifact.
//!
//! ```text
//! DATABASE_URL=postgres://…/scratch cargo test -j 2 -p backtest --test pg_fold_artifacts -- --ignored
//! ```
//! Needs migrations 0017, 0018, 0146 and 0189.

use backtest::models::RegistrySnapshot;
use chrono::{TimeZone, Utc};
use storage::postgres::models::record_fold_artifact;
use uuid::Uuid;

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations 0017, 0018, 0146 and 0189"]
async fn a_versions_folds_are_recorded_once_and_load_in_training_order() {
    let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connects");
    let model = format!("fold_test_{}", Uuid::new_v4().simple());
    let who = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ai_models (model_id, slug, display_name, model_kind, asset_class, \
           definition_json, created_by) \
         VALUES ($1, $1, $1, 'regressor', 'crypto_spot_cex', '{}'::jsonb, $2)",
    )
    .bind(&model)
    .bind(who)
    .execute(&pg)
    .await
    .unwrap();
    let through = |h: u32| Utc.with_ymd_and_hms(2025, 1, 1, h, 0, 0).unwrap();
    sqlx::query(
        "INSERT INTO model_versions (model_id, version, config_json, created_by, trained_through) \
         VALUES ($1, 1, '{}'::jsonb, $2, $3)",
    )
    .bind(&model)
    .bind(who)
    .bind(through(20))
    .execute(&pg)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO model_artifacts (model_id, version, storage_uri, artifact_type, size_bytes, sha256) \
         VALUES ($1, 1, 'file:///own', 'model', 1, 'own')",
    )
    .bind(&model)
    .execute(&pg)
    .await
    .unwrap();
    // Written out of order, and the second fold twice.
    for (fold, h) in [(1, 12), (0, 6), (1, 12)] {
        record_fold_artifact(
            &pg,
            &model,
            1,
            fold,
            through(h),
            &format!("file:///fold/{fold}"),
            1,
            &format!("sha{fold}"),
        )
        .await
        .unwrap();
    }

    let snap = RegistrySnapshot::load(&pg).await.unwrap();
    let e = snap.pinned(&model, 1).expect("the version loads");
    assert_eq!(
        e.artifact_uri.as_deref(),
        Some("file:///own"),
        "its own artifact is the `model` one"
    );
    let folds: Vec<(u32, &str)> = e
        .folds
        .iter()
        .map(|f| (f.index, f.artifact_uri.as_str()))
        .collect();
    assert_eq!(
        folds,
        [(0, "file:///fold/0"), (1, "file:///fold/1")],
        "once each, in training order"
    );
    assert_eq!(e.folds[1].trained_through, through(12));
    assert_eq!(e.folds[1].artifact_sha256, "sha1");

    // A fold row that names no training end is refused by the table.
    let refused = sqlx::query(
        "INSERT INTO model_artifacts (model_id, version, storage_uri, artifact_type, size_bytes, sha256, fold_index) \
         VALUES ($1, 1, 'file:///x', 'model_fold', 1, 'x', 2)",
    )
    .bind(&model)
    .execute(&pg)
    .await;
    assert!(refused.is_err(), "a fold without its training end");
}
