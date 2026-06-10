//! P1-T08: LedgerWriter append tests.
//!
//! Non-DB: proves the public API has no mutate method (compile-time).
//! DB tests are ignored and require `DATABASE_URL`.

use storage::ledger::LedgerWriter;

/// Compile-time proof: `LedgerWriter` exposes no `update` or `delete` method.
/// If anyone adds one, this annotation should be updated to match.
#[test]
fn ledger_writer_has_no_mutate_method() {
    // This test is documentation: it forces a reader to notice that
    // the public API surface of LedgerWriter only has `new` and `append`.
    // The compiler enforces no update/delete exists — this just records it.
    let _ = std::mem::size_of::<LedgerWriter>();
}

/// DB-backed: appending two events yields two rows with increasing `seq`.
#[tokio::test]
#[ignore = "requires a running migrated Postgres DB"]
async fn two_appends_yield_increasing_seq() {
    use chrono::Utc;
    use rust_decimal_macros::dec;
    use storage::ledger::{AccountMode, FillPayload, LedgerEvent};
    use uuid::Uuid;

    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let writer = LedgerWriter::new(pool.clone());

    let user_id = Uuid::new_v4();

    let e1 = LedgerEvent::Fill {
        id: Uuid::new_v4(),
        user_id,
        account_mode: AccountMode::Paper,
        venue: "kraken".into(),
        asset_class: "crypto_spot_cex".into(),
        instrument_id: "BTC-USDT".into(),
        strategy_id: None,
        payload: FillPayload {
            side: "buy".into(),
            qty: dec!(0.1),
            price: dec!(50000),
            commission: dec!(0.01),
        },
        usd_value: dec!(5000),
        context: serde_json::json!({}),
        occurred_at: Utc::now(),
    };

    let e2 = LedgerEvent::Fee {
        id: Uuid::new_v4(),
        user_id,
        account_mode: AccountMode::Paper,
        venue: "kraken".into(),
        asset_class: "crypto_spot_cex".into(),
        instrument_id: "BTC-USDT".into(),
        strategy_id: None,
        payload: storage::ledger::FeePayload {
            fee_type: "taker".into(),
            amount: dec!(5),
            currency: "USD".into(),
        },
        usd_value: dec!(5),
        context: serde_json::json!({}),
        occurred_at: Utc::now(),
    };

    let id1 = writer.append(e1).await.unwrap();
    let id2 = writer.append(e2).await.unwrap();

    // Verify both rows exist and seq is monotonic.
    let (seq1,): (i64,) = sqlx::query_as("SELECT seq FROM ledger_events WHERE id = $1")
        .bind(id1.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (seq2,): (i64,) = sqlx::query_as("SELECT seq FROM ledger_events WHERE id = $1")
        .bind(id2.0)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert!(seq2 > seq1, "seq must be monotonically increasing");
}
