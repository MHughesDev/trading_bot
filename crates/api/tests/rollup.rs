//! P4-T06 acceptance tests — USD rollup engine.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use storage::{ledger::AccountMode, pnl::FifoEngine};
use uuid::Uuid;

use api::rollup::{rollup_from_engine, MarkPrice};

fn uid() -> Uuid {
    Uuid::new_v4()
}
fn eid() -> Uuid {
    Uuid::new_v4()
}

fn venue_map(entries: &[(&str, &str, &str)]) -> HashMap<String, (String, String)> {
    entries
        .iter()
        .map(|(instrument, venue, asset_class)| {
            (
                instrument.to_string(),
                (venue.to_string(), asset_class.to_string()),
            )
        })
        .collect()
}

/// Rollup sums per-venue into per-class into platform total consistently.
#[test]
fn rollup_sums_tiers_consistently() {
    let user = uid();
    let mut engine = FifoEngine::new();
    let mode = AccountMode::Paper;

    // BTC: buy 1 @ 100, sell @ 200 → +100 realized
    engine.open_lot(
        user,
        mode,
        "BTC-USD",
        eid(),
        dec!(1),
        dec!(100),
        Decimal::ONE,
    );
    engine.close_lots(
        user,
        mode,
        "BTC-USD",
        eid(),
        dec!(1),
        dec!(200),
        Decimal::ONE,
    );

    // ETH: buy 2 @ 1000, hold (unrealized)
    engine.open_lot(
        user,
        mode,
        "ETH-USD",
        eid(),
        dec!(2),
        dec!(1000),
        Decimal::ONE,
    );

    let marks = vec![MarkPrice {
        instrument_id: "ETH-USD".to_owned(),
        mark: dec!(1100),
        usd_rate: Decimal::ONE,
    }];
    let vm = venue_map(&[
        ("BTC-USD", "coinbase", "crypto_spot_cex"),
        ("ETH-USD", "coinbase", "crypto_spot_cex"),
    ]);

    let rollup = rollup_from_engine(&engine, user, mode, &marks, &vm);

    assert_eq!(rollup.mode, "PAPER");
    // Platform realized = 100
    assert_eq!(rollup.realized_pnl_usd, dec!(100));
    // Platform unrealized = (1100-1000)*2 = 200
    assert_eq!(rollup.unrealized_pnl_usd, dec!(200));

    // Single asset class tile
    assert_eq!(rollup.by_asset_class.len(), 1);
    let ac_tile = &rollup.by_asset_class[0];
    assert_eq!(ac_tile.asset_class, "crypto_spot_cex");
    assert_eq!(ac_tile.realized_pnl_usd, dec!(100));

    // Single venue tile within the asset class
    assert_eq!(ac_tile.venues.len(), 1);
    let v_tile = &ac_tile.venues[0];
    assert_eq!(v_tile.venue, "coinbase");
    assert_eq!(v_tile.realized_pnl_usd, dec!(100));
}

/// Paper and Live are isolated — Live activity does not appear in Paper rollup.
#[test]
fn paper_and_live_are_isolated() {
    let user = uid();
    let mut engine = FifoEngine::new();

    // Paper: buy 1 BTC @ 100
    engine.open_lot(
        user,
        AccountMode::Paper,
        "BTC-USD",
        eid(),
        dec!(1),
        dec!(100),
        Decimal::ONE,
    );
    engine.close_lots(
        user,
        AccountMode::Paper,
        "BTC-USD",
        eid(),
        dec!(1),
        dec!(200),
        Decimal::ONE,
    );

    // Live: buy 1 ETH @ 1000
    engine.open_lot(
        user,
        AccountMode::Live,
        "ETH-USD",
        eid(),
        dec!(1),
        dec!(1000),
        Decimal::ONE,
    );
    engine.close_lots(
        user,
        AccountMode::Live,
        "ETH-USD",
        eid(),
        dec!(1),
        dec!(500),
        Decimal::ONE,
    );

    let vm = venue_map(&[
        ("BTC-USD", "coinbase", "crypto_spot_cex"),
        ("ETH-USD", "coinbase", "crypto_spot_cex"),
    ]);

    let paper_rollup = rollup_from_engine(&engine, user, AccountMode::Paper, &[], &vm);
    let live_rollup = rollup_from_engine(&engine, user, AccountMode::Live, &[], &vm);

    // Paper realized = +100; Live realized = -500
    assert_eq!(paper_rollup.realized_pnl_usd, dec!(100));
    assert_eq!(live_rollup.realized_pnl_usd, dec!(-500));

    // Paper win rate = 100%; Live win rate = 0%
    assert!((paper_rollup.win_rate - 1.0).abs() < 1e-9);
    assert!((live_rollup.win_rate - 0.0).abs() < 1e-9);
}

/// Win rate is position-level, not per-lot.
#[test]
fn win_rate_is_position_level() {
    let user = uid();
    let mut engine = FifoEngine::new();
    let mode = AccountMode::Paper;

    // BTC: open 2 lots, partial close at loss then profit — same position
    engine.open_lot(
        user,
        mode,
        "BTC-USD",
        eid(),
        dec!(1),
        dec!(100),
        Decimal::ONE,
    );
    engine.open_lot(
        user,
        mode,
        "BTC-USD",
        eid(),
        dec!(1),
        dec!(200),
        Decimal::ONE,
    );
    // Close both at 150: lot1 +50, lot2 -50 → net zero (counts as loss since <= 0)
    engine.close_lots(
        user,
        mode,
        "BTC-USD",
        eid(),
        dec!(2),
        dec!(150),
        Decimal::ONE,
    );

    // ETH: open 1 lot, close at profit
    engine.open_lot(
        user,
        mode,
        "ETH-USD",
        eid(),
        dec!(1),
        dec!(1000),
        Decimal::ONE,
    );
    engine.close_lots(
        user,
        mode,
        "ETH-USD",
        eid(),
        dec!(1),
        dec!(2000),
        Decimal::ONE,
    );

    let vm = venue_map(&[
        ("BTC-USD", "coinbase", "crypto_spot_cex"),
        ("ETH-USD", "coinbase", "crypto_spot_cex"),
    ]);
    let rollup = rollup_from_engine(&engine, user, mode, &[], &vm);

    // 1 win (ETH +1000), 1 non-win (BTC net 0) → win rate = 0.5
    // Note: BTC net realized = (150-100)*1 + (150-200)*1 = 50 - 50 = 0 → not positive → not a win
    assert!(
        (rollup.win_rate - 0.5).abs() < 1e-9,
        "win rate should be 0.5, got {}",
        rollup.win_rate
    );
}

/// Empty engine returns zeros.
#[test]
fn empty_engine_returns_zeros() {
    let user = uid();
    let engine = FifoEngine::new();
    let vm = HashMap::new();
    let rollup = rollup_from_engine(&engine, user, AccountMode::Paper, &[], &vm);
    assert_eq!(rollup.realized_pnl_usd, Decimal::ZERO);
    assert_eq!(rollup.unrealized_pnl_usd, Decimal::ZERO);
    assert!((rollup.win_rate - 0.0).abs() < 1e-9);
    assert!(rollup.by_asset_class.is_empty());
}
