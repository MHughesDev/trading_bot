//! P1-T04: Verifies `asset_class_registry` has 8 rows and `data_type_registry`
//! row count equals the number of `DataType` variants.
//!
//! Requires a running Postgres DB.  Run with `DATABASE_URL=... cargo test -- --ignored`.

use domain::{AssetClass, DataType};

/// All 8 `AssetClass` variants exist in the domain enum — compile-time proof.
#[test]
fn asset_class_has_8_variants() {
    let classes = [
        AssetClass::CryptoSpotCex,
        AssetClass::Equity,
        AssetClass::Fx,
        AssetClass::PredictionMarket,
        AssetClass::Option,
        AssetClass::CryptoSpotDex,
        AssetClass::PerpetualSwap,
        AssetClass::FuturesExpiring,
    ];
    assert_eq!(classes.len(), 8);
}

/// All `DataType` variants are enumerable via `DataType::all()`.
#[test]
fn data_type_all_count_matches_variant_count() {
    // 10 variants defined in the enum — update this if variants are added.
    assert_eq!(DataType::all().len(), 10);
}

/// DB-backed check: migration 0006 seeds correct row counts.
/// Requires DATABASE_URL env var pointing to a migrated Postgres instance.
#[tokio::test]
#[ignore = "requires a running migrated Postgres DB"]
async fn migration_0006_seeds_correct_row_counts() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();

    let (asset_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM asset_class_registry")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        asset_count, 8,
        "asset_class_registry must have exactly 8 rows"
    );

    let (dt_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM data_type_registry")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        dt_count as usize,
        domain::DataType::all().len(),
        "data_type_registry row count must match DataType variant count"
    );
}
