//! Set E audit item #5: paper accounts survive a restart.
//!
//! Two engines stand in for two processes: the first trades and its saver
//! writes `paper_account_state`; the second is a fresh boot that restores from
//! the table and must hold the same cash, positions and realized P&L.
//!
//! Needs `DATABASE_URL` pointing at a database migrated through 0153:
//!   DATABASE_URL=postgres://…/scratch cargo test -j 2 -p execution \
//!     --test paper_state_durable -- --ignored

use std::collections::HashMap;
use std::str::FromStr;

use domain::{
    instrument::AssetClass,
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::paper::{persist, PaperTradingEngine};
use rust_decimal::Decimal;

/// Both tests own the whole `paper_account_state` table; run them one at a time.
static TABLE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connect")
}

fn market(instrument: &str, side: Side, qty: &str) -> OrderIntent {
    OrderIntent::new(
        instrument,
        side,
        OrderType::Market,
        Size::from_str(qty).unwrap(),
        None,
        None,
    )
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0153"]
async fn a_restart_restores_cash_positions_and_realized_pnl() {
    let _table = TABLE.lock().await;
    let pg = pool().await;
    sqlx::query("DELETE FROM paper_account_state")
        .execute(&pg)
        .await
        .unwrap();

    // ── process 1 trades ──
    let before = PaperTradingEngine::new();
    before.on_mark("BTC-USD", Price::from_str("50000").unwrap());
    before
        .submit(
            AssetClass::CryptoSpotCex,
            &market("BTC-USD", Side::Buy, "1"),
        )
        .unwrap();
    before.on_mark("BTC-USD", Price::from_str("52000").unwrap());
    before
        .submit(
            AssetClass::CryptoSpotCex,
            &market("BTC-USD", Side::Sell, "0.5"),
        )
        .unwrap();

    let mut last = HashMap::new();
    let mut generation = 0;
    let written = persist::save_dirty(&before, &pg, &mut last, &mut generation)
        .await
        .expect("saved");
    assert!(written >= 1, "the account that traded was written");

    // A pass with nothing new writes nothing.
    let _ = before.snapshot(AssetClass::CryptoSpotCex); // a read takes the lock
    assert_eq!(
        persist::save_dirty(&before, &pg, &mut last, &mut generation)
            .await
            .unwrap(),
        0,
        "a read-only touch must not rewrite the row"
    );

    // ── process 2 boots ──
    let after = PaperTradingEngine::new();
    after.on_mark("BTC-USD", Price::from_str("52000").unwrap());
    let fresh = after.snapshot(AssetClass::CryptoSpotCex);
    let restored = persist::restore(&after, &pg).await.expect("restored");
    assert!(restored >= 1);

    let (a, b) = (
        before.snapshot(AssetClass::CryptoSpotCex),
        after.snapshot(AssetClass::CryptoSpotCex),
    );
    assert_ne!(
        fresh.cash, a.cash,
        "the test must trade enough to tell a reseed apart"
    );
    assert_eq!(b.cash, a.cash, "cash survived the restart");
    assert_eq!(
        b.realized_pnl, a.realized_pnl,
        "realized P&L survived the restart"
    );
    assert_eq!(b.fees_paid, a.fees_paid);
    assert_eq!(b.positions.len(), 1);
    assert_eq!(
        b.positions[0].quantity,
        Decimal::new(5, 1),
        "the open position survived"
    );
    assert_eq!(
        before.daily_loss_usd(AssetClass::CryptoSpotCex),
        after.daily_loss_usd(AssetClass::CryptoSpotCex),
        "the gate's daily-loss input survived the restart"
    );
    assert_eq!(
        before.export_account(AssetClass::CryptoSpotCex),
        after.export_account(AssetClass::CryptoSpotCex),
        "the whole account, ledger included, is identical"
    );
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations through 0153"]
async fn an_unknown_state_version_refuses_to_boot_rather_than_reseeding() {
    let _table = TABLE.lock().await;
    let pg = pool().await;
    let mut tx = pg.begin().await.unwrap();
    // As the platform's role, so a missing grant fails here too.
    sqlx::query("SET LOCAL ROLE app_role")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("DELETE FROM paper_account_state")
        .execute(&mut *tx)
        .await
        .expect("app_role can delete paper state");
    sqlx::query(
        "INSERT INTO paper_account_state (asset_class, state_version, state, generation) \
         VALUES ('fx', 99, '{\"asset_class\":\"fx\"}', 0)",
    )
    .execute(&mut *tx)
    .await
    .expect("app_role can write paper state");
    tx.commit().await.unwrap();

    let engine = PaperTradingEngine::new();
    let err = persist::restore(&engine, &pg).await.unwrap_err();
    assert!(
        matches!(err, persist::PersistError::UnknownVersion { found: 99, .. }),
        "{err}"
    );

    sqlx::query("DELETE FROM paper_account_state")
        .execute(&pg)
        .await
        .unwrap();
}
