//! Paper-mode dashboard rollup — built directly from the internal
//! [`PaperTradingEngine`], never from venue APIs or the ledger tables.
//!
//! Paper mode has no venues: the internal engine is the only execution
//! surface.  The rollup therefore carries **asset-class-level account data**
//! (cash, equity, margin, positions) plus **bot-wide totals**, and leaves the
//! per-venue tile list empty.

use execution::paper::{PaperAccountSnapshot, PaperTradingEngine};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{AssetClassTile, RollupResponse};

/// Account-level data for one paper asset-class account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperAccountInfo {
    pub currency: String,
    pub cash: Decimal,
    pub equity: Decimal,
    pub used_margin: Decimal,
    pub free_collateral: Decimal,
    pub fees_paid: Decimal,
    pub open_positions: usize,
    pub positions: Vec<PaperPositionInfo>,
}

/// One open paper position, mark-resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperPositionInfo {
    pub instrument_id: String,
    pub quantity: Decimal,
    pub average_entry_price: Decimal,
    pub mark_price: Option<Decimal>,
    pub unrealized_pnl: Decimal,
    pub notional: Decimal,
}

/// Bot-wide paper account totals across all asset classes.
///
/// Non-USD accounts are converted 1:1 for stablecoins (USDC); accounts in
/// other currencies (e.g. the ETH-denominated NFT account) are excluded from
/// the USD totals and listed in `excluded_currencies`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperAccountTotals {
    pub equity_usd: Decimal,
    pub cash_usd: Decimal,
    pub realized_pnl_usd: Decimal,
    pub unrealized_pnl_usd: Decimal,
    pub fees_paid_usd: Decimal,
    pub open_positions: usize,
    pub excluded_currencies: Vec<String>,
}

fn usd_convertible(currency: &str) -> bool {
    matches!(currency, "USD" | "USDC")
}

fn tile_from_snapshot(snap: &PaperAccountSnapshot) -> AssetClassTile {
    let unrealized: Decimal = snap.positions.iter().map(|p| p.unrealized_pnl).sum();
    let win_rate = if snap.closed_trades == 0 {
        0.0
    } else {
        snap.winning_trades as f64 / snap.closed_trades as f64
    };
    AssetClassTile {
        asset_class: snap.asset_class.as_str().to_owned(),
        realized_pnl_usd: snap.realized_pnl,
        unrealized_pnl_usd: unrealized,
        win_rate,
        // Paper mode has no venues — execution is internal.
        venues: vec![],
        account: Some(PaperAccountInfo {
            currency: snap.currency.to_owned(),
            cash: snap.cash,
            equity: snap.equity,
            used_margin: snap.used_margin,
            free_collateral: snap.free_collateral,
            fees_paid: snap.fees_paid,
            open_positions: snap.positions.len(),
            positions: snap
                .positions
                .iter()
                .map(|p| PaperPositionInfo {
                    instrument_id: p.instrument_id.clone(),
                    quantity: p.quantity,
                    average_entry_price: p.average_entry_price,
                    mark_price: p.mark_price,
                    unrealized_pnl: p.unrealized_pnl,
                    notional: p.notional,
                })
                .collect(),
        }),
    }
}

/// Build the full paper-mode rollup from the engine's per-class snapshots.
pub fn paper_rollup(engine: &PaperTradingEngine) -> RollupResponse {
    let snapshots = engine.snapshots();

    let mut totals = PaperAccountTotals {
        equity_usd: Decimal::ZERO,
        cash_usd: Decimal::ZERO,
        realized_pnl_usd: Decimal::ZERO,
        unrealized_pnl_usd: Decimal::ZERO,
        fees_paid_usd: Decimal::ZERO,
        open_positions: 0,
        excluded_currencies: vec![],
    };
    let mut wins: u64 = 0;
    let mut closes: u64 = 0;

    let tiles: Vec<AssetClassTile> = snapshots
        .iter()
        .map(|snap| {
            wins += snap.winning_trades;
            closes += snap.closed_trades;
            totals.open_positions += snap.positions.len();
            if usd_convertible(snap.currency) {
                totals.equity_usd += snap.equity;
                totals.cash_usd += snap.cash;
                totals.realized_pnl_usd += snap.realized_pnl;
                totals.unrealized_pnl_usd += snap
                    .positions
                    .iter()
                    .map(|p| p.unrealized_pnl)
                    .sum::<Decimal>();
                totals.fees_paid_usd += snap.fees_paid;
            } else if !totals
                .excluded_currencies
                .iter()
                .any(|c| c == snap.currency)
            {
                totals.excluded_currencies.push(snap.currency.to_owned());
            }
            tile_from_snapshot(snap)
        })
        .collect();

    RollupResponse {
        mode: "paper",
        realized_pnl_usd: totals.realized_pnl_usd,
        unrealized_pnl_usd: totals.unrealized_pnl_usd,
        win_rate: if closes == 0 {
            0.0
        } else {
            wins as f64 / closes as f64
        },
        by_asset_class: tiles,
        account_totals: Some(totals),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::instrument::AssetClass;
    use domain::money::{Price, Size};
    use domain::order::{OrderIntent, OrderType, Side};
    use rust_decimal_macros::dec;

    fn market(instrument: &str, side: Side, qty: Decimal) -> OrderIntent {
        OrderIntent::new(
            instrument,
            side,
            OrderType::Market,
            Size::from_decimal(qty),
            None,
            None,
        )
    }

    #[test]
    fn paper_rollup_carries_account_data_and_no_venues() {
        let engine = PaperTradingEngine::new();
        engine.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        engine
            .submit(
                AssetClass::CryptoSpotCex,
                &market("BTC-USD", Side::Buy, dec!(1)),
            )
            .unwrap();

        let rollup = paper_rollup(&engine);
        assert_eq!(rollup.mode, "paper");
        assert!(rollup.account_totals.is_some());

        // Every asset class is represented and none has venue tiles.
        assert_eq!(rollup.by_asset_class.len(), 11);
        assert!(rollup.by_asset_class.iter().all(|t| t.venues.is_empty()));

        let crypto = rollup
            .by_asset_class
            .iter()
            .find(|t| t.asset_class == "crypto_spot_cex")
            .expect("crypto tile present");
        let account = crypto.account.as_ref().expect("account data present");
        assert!(account.cash < dec!(100_000), "cash debited by the buy");
        assert_eq!(account.open_positions, 1);
        assert_eq!(account.positions[0].instrument_id, "BTC-USD");
    }

    #[test]
    fn totals_exclude_non_usd_accounts() {
        let engine = PaperTradingEngine::new();
        let rollup = paper_rollup(&engine);
        let totals = rollup.account_totals.unwrap();
        // The NFT account is ETH-denominated and must not pollute USD totals.
        assert!(totals.excluded_currencies.contains(&"ETH".to_owned()));
        // Seeds: 8 USD accounts × 100k + prediction 10k + DEX 100k USDC.
        assert_eq!(totals.cash_usd, dec!(910_000));
        assert_eq!(totals.equity_usd, totals.cash_usd);
    }

    #[test]
    fn win_rate_reflects_closed_trades() {
        let engine = PaperTradingEngine::new();
        engine.on_mark("AAPL", Price::from_decimal(dec!(100)));
        engine
            .submit(AssetClass::Equity, &market("AAPL", Side::Buy, dec!(10)))
            .unwrap();
        engine.on_mark("AAPL", Price::from_decimal(dec!(120)));
        engine
            .submit(AssetClass::Equity, &market("AAPL", Side::Sell, dec!(10)))
            .unwrap();

        let rollup = paper_rollup(&engine);
        let equity = rollup
            .by_asset_class
            .iter()
            .find(|t| t.asset_class == "equity")
            .unwrap();
        assert!(equity.realized_pnl_usd > Decimal::ZERO);
        assert!((equity.win_rate - 1.0).abs() < f64::EPSILON);
    }
}
