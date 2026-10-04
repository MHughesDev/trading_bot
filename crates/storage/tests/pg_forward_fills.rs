//! Forward marks and fills round-trip (Set T T-0.4; migration 0186).
//!
//! Needs a database with 0186 applied and a `mlops.paper_deployment` row to
//! hang them on. A scratch database with only the parent key is enough:
//!
//! ```text
//! CREATE SCHEMA mlops;
//! CREATE TABLE mlops.paper_deployment (deployment_id uuid PRIMARY KEY DEFAULT gen_random_uuid());
//! psql -f migrations/0186_strategy_forward_fills.sql
//! DATABASE_URL=… FORWARD_DEPLOYMENT_ID=<a paper_deployment id> \
//!   cargo test -j 2 -p storage --test pg_forward_fills -- --ignored
//! ```

use chrono::{Duration, Utc};
use rust_decimal_macros::dec;
use storage::postgres::decision_journal as journal;

#[tokio::test]
#[ignore = "needs DATABASE_URL with 0186 and FORWARD_DEPLOYMENT_ID"]
async fn marks_and_fills_come_back_in_order_with_an_unknown_fee_kept_unknown() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let id: uuid::Uuid = std::env::var("FORWARD_DEPLOYMENT_ID")
        .expect("FORWARD_DEPLOYMENT_ID must be set")
        .parse()
        .unwrap();
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let base = Utc::now() + Duration::days(i64::from(rand_day()));
    for (i, kind) in ["open", "close", "close"].iter().enumerate() {
        journal::record_forward_mark(
            &pool,
            &journal::ForwardMark {
                forward_deployment_id: id,
                tenant_id: "t".into(),
                bar: base + Duration::minutes(i as i64),
                mark: dec!(100) + rust_decimal::Decimal::from(i as i64),
                kind: (*kind).into(),
            },
        )
        .await
        .unwrap();
    }
    let start = journal::forward_fills(&pool, id).await.unwrap().len() as i64;
    for (k, charge) in [Some(dec!(0.01)), None].into_iter().enumerate() {
        journal::record_forward_fill(
            &pool,
            &journal::ForwardFill {
                forward_deployment_id: id,
                tenant_id: "t".into(),
                seq: start + k as i64 + 1,
                side: "buy".into(),
                filled: dec!(0.5),
                fill_price: dec!(101.25),
                charge,
                decision_price: dec!(101),
                filled_at: base,
            },
        )
        .await
        .unwrap();
    }
    let marks = journal::forward_marks(&pool, id).await.unwrap();
    assert!(marks.windows(2).all(|w| w[0].bar < w[1].bar));
    let fills = journal::forward_fills(&pool, id).await.unwrap();
    let tail = &fills[fills.len() - 2..];
    assert_eq!(tail[0].charge, Some(dec!(0.01)));
    assert_eq!(tail[1].charge, None, "an unreported fee stays unknown, never zero");
    assert_eq!(tail[1].fill_price, dec!(101.25));
}

/// A different day per run, so marks never collide on their key.
fn rand_day() -> u32 {
    u32::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            % 10_000,
    )
    .unwrap()
}
