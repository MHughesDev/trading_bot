//! The instrument lookup against the table migration 0002 actually creates.
//!
//! Until 2026-10-03 `fetch_by_id` selected columns that do not exist (`id`,
//! `is_active`) and decoded an INT4 as i64, so every v3 backtest, preview and
//! series run failed on a real database while every unit test passed. Found by
//! the v3 migration's re-run (Set U U-8.5).
//!
//! ```text
//! DATABASE_URL=… cargo test -j 2 -p storage --test pg_instruments -- --ignored
//! ```

use storage::postgres::instruments;

#[tokio::test]
#[ignore = "needs DATABASE_URL with migration 0002"]
async fn an_instrument_is_found_by_its_id_with_its_sizes() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = sqlx::PgPool::connect(&url).await.expect("connects");
    let id = format!("PGI-{}-USD", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO instruments (instrument_id, asset_class, venue_id, base_precision, \
         quote_precision, tick_size, lot_size, trading_hours_json, halt_policy, trust_tier) \
         VALUES ($1, 'crypto_spot_cex', 'coinbase', 8, 2, 0.01, 0.0001, '{}', 'non_haltable', \
         'centralized_exchange')",
    )
    .bind(&id)
    .execute(&pool)
    .await
    .expect("insert");
    let got = instruments::fetch_by_id(&pool, &id)
        .await
        .expect("the query runs against the real columns")
        .expect("found");
    assert_eq!(got.tick_size.normalize().to_string(), "0.01");
    assert_eq!(got.lot_size.normalize().to_string(), "0.0001");
    assert!(instruments::list_active(&pool)
        .await
        .expect("lists")
        .iter()
        .any(|i| i.instrument_id == id));
    assert!(instruments::fetch_by_id(&pool, "NO-SUCH-USD")
        .await
        .expect("runs")
        .is_none());
    // An instrument row is never deleted (migration 0155, P-4.4); the fixture is delisted
    // instead. Its id is unique per run, so the row cannot collide with the next one.
    sqlx::query(
        "UPDATE instruments SET active = false, status = 'delisted', delisted_at = now(), delisting_reason = 'pg_instruments test fixture' WHERE instrument_id = $1",
    )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
}
