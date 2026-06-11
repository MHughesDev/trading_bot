//! P4-T05 acceptance tests — FIFO P&L lot engine.

use domain::order::Side;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use storage::{ledger::AccountMode, pnl::FifoEngine};
use uuid::Uuid;

fn uid() -> Uuid {
    Uuid::new_v4()
}
fn eid() -> Uuid {
    Uuid::new_v4()
}

/// Two opens + one partial close realize USD P&L against the oldest lot first.
#[test]
fn two_opens_partial_close_realizes_fifo() {
    let user = uid();
    let mut engine = FifoEngine::new();
    let mode = AccountMode::Paper;

    // Lot 1: buy 2 BTC @ $30_000
    engine.open_lot(
        user,
        mode,
        "BTC-USD",
        eid(),
        Side::Buy,
        dec!(2),
        dec!(30000),
        Decimal::ONE,
    );
    // Lot 2: buy 1 BTC @ $40_000
    engine.open_lot(
        user,
        mode,
        "BTC-USD",
        eid(),
        Side::Buy,
        dec!(1),
        dec!(40000),
        Decimal::ONE,
    );

    // Close 2 BTC @ $35_000 — consumes lot 1 entirely (FIFO)
    let closes = engine.close_lots(
        user,
        mode,
        "BTC-USD",
        eid(),
        dec!(2),
        dec!(35000),
        Decimal::ONE,
    );

    assert_eq!(closes.len(), 1, "one close for one lot fully consumed");
    // realized = (35000 - 30000) * 2 * 1 = 10_000
    assert_eq!(closes[0].realized_usd, dec!(10000));
    assert_eq!(closes[0].close_qty, dec!(2));
}

/// Remaining unrealized P&L reflects the current mark.
#[test]
fn remaining_lot_unrealized_at_mark() {
    let user = uid();
    let mut engine = FifoEngine::new();
    let mode = AccountMode::Paper;

    // Buy 3 ETH @ $2000
    engine.open_lot(
        user,
        mode,
        "ETH-USD",
        eid(),
        Side::Buy,
        dec!(3),
        dec!(2000),
        Decimal::ONE,
    );
    // Sell 1 ETH @ $2500
    engine.close_lots(
        user,
        mode,
        "ETH-USD",
        eid(),
        dec!(1),
        dec!(2500),
        Decimal::ONE,
    );

    // 2 ETH remain; mark is $2600
    let upnl = engine.unrealized_usd(user, mode, "ETH-USD", dec!(2600), Decimal::ONE);
    // (2600 - 2000) * 2 = 1200
    assert_eq!(upnl, dec!(1200));
}

/// A position that closes net-positive counts toward win rate; net-negative does not.
#[test]
fn win_rate_counts_profitable_positions_only() {
    let user = uid();
    let mut engine = FifoEngine::new();
    let mode = AccountMode::Paper;

    // BTC: open 1 @ 100, close @ 200 → +100 (win)
    engine.open_lot(
        user,
        mode,
        "BTC-USD",
        eid(),
        Side::Buy,
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

    // ETH: open 1 @ 2000, close @ 1800 → −200 (loss)
    engine.open_lot(
        user,
        mode,
        "ETH-USD",
        eid(),
        Side::Buy,
        dec!(1),
        dec!(2000),
        Decimal::ONE,
    );
    engine.close_lots(
        user,
        mode,
        "ETH-USD",
        eid(),
        dec!(1),
        dec!(1800),
        Decimal::ONE,
    );

    let wr = engine.win_rate(user, mode);
    // 1 win / 2 total = 50%
    assert!((wr - 0.5).abs() < 1e-9, "win rate = 0.5, got {wr}");
}

/// FIFO respects lot ordering across three lots.
#[test]
fn fifo_order_respected_across_three_lots() {
    let user = uid();
    let mut engine = FifoEngine::new();
    let mode = AccountMode::Paper;

    engine.open_lot(
        user,
        mode,
        "SOL-USD",
        eid(),
        Side::Buy,
        dec!(1),
        dec!(100),
        Decimal::ONE,
    );
    engine.open_lot(
        user,
        mode,
        "SOL-USD",
        eid(),
        Side::Buy,
        dec!(1),
        dec!(200),
        Decimal::ONE,
    );
    engine.open_lot(
        user,
        mode,
        "SOL-USD",
        eid(),
        Side::Buy,
        dec!(1),
        dec!(300),
        Decimal::ONE,
    );

    // Close 1.5 SOL @ $150 — consumes lot1 (1 @ 100) fully, then 0.5 of lot2 (0.5 @ 200).
    let closes = engine.close_lots(
        user,
        mode,
        "SOL-USD",
        eid(),
        dec!(1.5),
        dec!(150),
        Decimal::ONE,
    );

    assert_eq!(closes.len(), 2, "spans two lots");
    // First close: lot1 fully consumed.
    assert_eq!(closes[0].close_qty, dec!(1));
    assert_eq!(closes[0].realized_usd, dec!(50)); // (150-100)*1 = 50
                                                  // Second close: 0.5 of lot2.
    assert_eq!(closes[1].close_qty, dec!(0.5));
    assert_eq!(closes[1].realized_usd, dec!(-25)); // (150-200)*0.5 = -25
}
