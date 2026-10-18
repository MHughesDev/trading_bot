//! Set U U-4.7: a deployment's own paper account survives a restart.
//!
//! Two engines stand in for two processes: the first opens a deployment's
//! account, trades in it, and the saver writes `paper_deployment_account_state`
//! (0188); the second is a fresh boot that restores before any order — and
//! opening the account again then keeps what was restored rather than
//! reseeding it.
//!
//! Needs `DATABASE_URL` at a database migrated through 0142, 0153 and 0188:
//!   DATABASE_URL=postgres://…/scratch cargo test -j 2 -p execution \
//!     --test paper_deployment_accounts_durable -- --ignored

use std::collections::HashMap;
use std::str::FromStr;

use domain::{
    instrument::AssetClass,
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::paper::{persist, PaperTradingEngine};
use rust_decimal::Decimal;
use uuid::Uuid;

fn market(instrument: &str, side: Side, qty: &str, deployment: Uuid) -> OrderIntent {
    OrderIntent::new(
        instrument,
        side,
        OrderType::Market,
        Size::from_str(qty).unwrap(),
        None,
        Some(deployment.to_string()),
    )
}

/// The account's FK: a recorded deployment.
async fn record(pg: &sqlx::PgPool, id: Uuid) {
    sqlx::query(
        "INSERT INTO strategy_deployment (deployment_id, user_id, strategy_name, revision_hash, \
           definition_json, instrument_id, environment, venue, starting_capital, started_by) \
         VALUES ($1, 'u-acct', 'acct_test', repeat('a', 64), '{}'::jsonb, $2, 'paper', \
           'platform_paper', 10000, 'u-acct')",
    )
    .bind(id)
    .bind(format!("ACCT-{}-USD", id.simple()))
    .execute(pg)
    .await
    .expect("record the deployment");
}

#[tokio::test]
#[ignore = "needs DATABASE_URL with migrations 0142, 0153 and 0188"]
async fn a_restart_restores_each_deployments_own_account() {
    let pg = sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL"))
        .await
        .expect("connect");
    let ac = AssetClass::CryptoSpotCex;
    let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
    record(&pg, a).await;
    record(&pg, b).await;

    // ── process 1 ──
    let before = PaperTradingEngine::new();
    before.open_deployment_account(a, ac, Decimal::from(10_000)).unwrap();
    before.open_deployment_account(b, ac, Decimal::from(5_000)).unwrap();
    before.on_mark("BTC-USD", Price::from_str("100").unwrap());
    before.submit(ac, &market("BTC-USD", Side::Buy, "3", a)).unwrap();
    before.submit(ac, &market("BTC-USD", Side::Buy, "1", b)).unwrap();
    let mut last = HashMap::new();
    let written = persist::save_dirty_deployments(&before, &pg, &mut last)
        .await
        .unwrap();
    assert!(written >= 2, "both accounts written ({written})");
    // Nothing changed: nothing rewritten.
    assert_eq!(
        persist::save_dirty_deployments(&before, &pg, &mut last).await.unwrap(),
        0
    );

    // ── process 2 ──
    let after = PaperTradingEngine::new();
    persist::restore(&after, &pg).await.unwrap();
    // The runner opens its account at start: the restored one stays.
    assert!(!after.open_deployment_account(a, ac, Decimal::from(10_000)).unwrap());
    for (id, qty) in [(a, "3"), (b, "1")] {
        let s = id.to_string();
        let pos = after
            .positions_for(ac, Some(&s))
            .into_iter()
            .find(|p| p.instrument_id == "BTC-USD")
            .expect("position restored");
        assert_eq!(pos.quantity, Decimal::from_str(qty).unwrap());
        assert_eq!(
            after.snapshot_for(ac, Some(&s)).cash,
            before.snapshot_for(ac, Some(&s)).cash,
            "cash restored exactly"
        );
    }
}
