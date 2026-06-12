//! CLOB paper fill simulator — used for crypto spot (CEX), perps, futures, FX.
//!
//! Market orders fill at `mark ± slippage_ticks * tick_size`.
//! Limit orders fill only when the mark price crosses the limit (rising-edge check).

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use domain::{
    money::Price,
    order::{OrderIntent, OrderType, Side},
};

use super::{PaperFill, PaperFillSimulator};

/// Configurable spread, slippage, and size impact for CLOB fills.
#[derive(Debug, Clone)]
pub struct ClobFillSimulator {
    /// Half-spread in basis points applied to the mark price.
    pub half_spread_bps: Decimal,
    /// Additional slippage in ticks (applied for market orders).
    pub slippage_ticks: Decimal,
    /// Tick size for slippage calculation.
    pub tick_size: Decimal,
    /// Maker/taker fee rate (fraction, e.g. 0.001 = 0.1%).
    pub fee_rate: Decimal,
    /// Fraction of requested qty that fills immediately (1 = full, 0.5 = half).
    /// Models shallow order-book depth for deterministic partial-fill testing.
    pub partial_fill_ratio: Decimal,
    /// Size-impact model: notional (quote currency) at which a market order
    /// pays `impact_bps_at_depth` of extra slippage.  Impact scales linearly
    /// with notional and is capped at `max_impact_bps`.  `None` disables.
    pub depth_notional: Option<Decimal>,
    /// Extra slippage (bps) paid by a market order of exactly `depth_notional`.
    pub impact_bps_at_depth: Decimal,
    /// Upper bound on size impact (bps), however large the order.
    pub max_impact_bps: Decimal,
}

impl Default for ClobFillSimulator {
    fn default() -> Self {
        Self {
            half_spread_bps: dec!(5), // 0.05%
            slippage_ticks: dec!(1),
            tick_size: dec!(0.01),
            fee_rate: dec!(0.001), // 0.1%
            partial_fill_ratio: Decimal::ONE,
            depth_notional: None,
            impact_bps_at_depth: Decimal::ZERO,
            max_impact_bps: Decimal::ZERO,
        }
    }
}

impl ClobFillSimulator {
    /// Compute the simulated fill price for a **market** order, incorporating
    /// half-spread, tick slippage, and size impact in the trade's direction.
    fn market_fill_price(&self, mark: Price, side: Side, qty: Decimal) -> Price {
        let m = mark.inner();
        let spread_adj = m * self.half_spread_bps / dec!(10000);
        let slip_adj = self.slippage_ticks * self.tick_size;
        let impact_adj = m * self.impact_bps(m, qty) / dec!(10000);
        let total_adj = spread_adj + slip_adj + impact_adj;
        let raw = match side {
            Side::Buy => m + total_adj,
            Side::Sell => m - total_adj,
        };
        Price::from_decimal(raw.max(Decimal::ZERO))
    }

    /// Extra slippage (bps) for a market order of `qty` at mark `m`:
    /// `impact_bps_at_depth × (qty × m / depth_notional)`, capped.
    fn impact_bps(&self, m: Decimal, qty: Decimal) -> Decimal {
        let Some(depth) = self.depth_notional else {
            return Decimal::ZERO;
        };
        if depth <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        (self.impact_bps_at_depth * (qty * m) / depth).min(self.max_impact_bps)
    }

    /// Returns `true` if a resting limit order would fill at the current mark.
    ///
    /// - Buy limit: fills when mark <= limit_price (market moved down to limit).
    /// - Sell limit: fills when mark >= limit_price (market moved up to limit).
    pub fn limit_fills_at_mark(&self, limit: Price, mark: Price, side: Side) -> bool {
        match side {
            Side::Buy => mark.inner() <= limit.inner(),
            Side::Sell => mark.inner() >= limit.inner(),
        }
    }
}

impl PaperFillSimulator for ClobFillSimulator {
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill {
        let (fill_price, fills) = match intent.order_type {
            OrderType::Market => {
                let fp = self.market_fill_price(mark, intent.side, intent.size.inner());
                (fp, true)
            }
            OrderType::Limit | OrderType::StopLimit => {
                if let Some(limit) = intent.limit_price {
                    if self.limit_fills_at_mark(limit, mark, intent.side) {
                        // Fill at the limit price (price improvement not modelled).
                        (limit, true)
                    } else {
                        // Resting — no fill yet; return zero-qty fill as sentinel.
                        (limit, false)
                    }
                } else {
                    // No limit price provided — treat as market.
                    (
                        self.market_fill_price(mark, intent.side, intent.size.inner()),
                        true,
                    )
                }
            }
        };

        let qty = if fills {
            (intent.size.inner() * self.partial_fill_ratio).max(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };
        let fee = qty * fill_price.inner() * self.fee_rate;

        PaperFill {
            idempotency_key: intent.idempotency_key,
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_qty: qty,
            fill_price,
            fee,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::Size;
    use domain::order::{OrderIntent, OrderType, Side};

    fn intent(order_type: OrderType, side: Side, limit: Option<Price>) -> OrderIntent {
        let mut i = OrderIntent::new(
            "BTC-USDT",
            side,
            order_type,
            Size::from_decimal(dec!(1)),
            limit,
            None,
        );
        // Stable key for tests.
        i.idempotency_key = uuid::Uuid::nil();
        i
    }

    #[test]
    fn market_buy_fills_above_mark() {
        let sim = ClobFillSimulator::default();
        let mark = Price::from_decimal(dec!(50000));
        let fill = sim.simulate_fill(&intent(OrderType::Market, Side::Buy, None), mark);
        assert!(
            fill.filled_qty > Decimal::ZERO,
            "market buy must fill immediately"
        );
        assert!(
            fill.fill_price.inner() > mark.inner(),
            "buy fill must be above mark (slippage)"
        );
    }

    #[test]
    fn market_sell_fills_below_mark() {
        let sim = ClobFillSimulator::default();
        let mark = Price::from_decimal(dec!(50000));
        let fill = sim.simulate_fill(&intent(OrderType::Market, Side::Sell, None), mark);
        assert!(fill.filled_qty > Decimal::ZERO);
        assert!(fill.fill_price.inner() < mark.inner());
    }

    #[test]
    fn larger_market_orders_pay_more_impact() {
        let sim = ClobFillSimulator {
            depth_notional: Some(dec!(1_000_000)),
            impact_bps_at_depth: dec!(10),
            max_impact_bps: dec!(50),
            ..Default::default()
        };
        let mark = Price::from_decimal(dec!(50_000));
        let mut small = intent(OrderType::Market, Side::Buy, None);
        small.size = Size::from_decimal(dec!(0.1)); // $5k notional
        let mut big = intent(OrderType::Market, Side::Buy, None);
        big.size = Size::from_decimal(dec!(20)); // $1M notional
        let mut huge = intent(OrderType::Market, Side::Buy, None);
        huge.size = Size::from_decimal(dec!(1_000)); // $50M — capped

        let p_small = sim.simulate_fill(&small, mark).fill_price.inner();
        let p_big = sim.simulate_fill(&big, mark).fill_price.inner();
        let p_huge = sim.simulate_fill(&huge, mark).fill_price.inner();

        assert!(p_big > p_small, "larger order must fill at a worse price");
        // At depth: +10 bps over the small order's near-zero impact.
        assert!(p_big - p_small > dec!(45) && p_big - p_small < dec!(55));
        // Cap: 50 bps max impact = mark × 0.005 over spread+slippage.
        let base = sim.simulate_fill(&small, mark).fill_price.inner();
        assert!(p_huge - base <= dec!(250.01), "impact must respect the cap");
    }

    #[test]
    fn resting_buy_limit_fills_only_when_mark_at_or_below_limit() {
        let sim = ClobFillSimulator::default();
        let limit = Price::from_decimal(dec!(49900));

        // Mark above limit — should NOT fill.
        let mark_above = Price::from_decimal(dec!(50000));
        let no_fill = sim.simulate_fill(
            &intent(OrderType::Limit, Side::Buy, Some(limit)),
            mark_above,
        );
        assert_eq!(
            no_fill.filled_qty,
            Decimal::ZERO,
            "should not fill above limit"
        );

        // Mark at limit — should fill.
        let mark_at = Price::from_decimal(dec!(49900));
        let fill = sim.simulate_fill(&intent(OrderType::Limit, Side::Buy, Some(limit)), mark_at);
        assert!(fill.filled_qty > Decimal::ZERO, "should fill at limit");
    }

    #[test]
    fn resting_sell_limit_fills_only_when_mark_at_or_above_limit() {
        let sim = ClobFillSimulator::default();
        let limit = Price::from_decimal(dec!(50100));

        // Mark below limit — should NOT fill.
        let mark_below = Price::from_decimal(dec!(50000));
        let no_fill = sim.simulate_fill(
            &intent(OrderType::Limit, Side::Sell, Some(limit)),
            mark_below,
        );
        assert_eq!(no_fill.filled_qty, Decimal::ZERO);

        // Mark at limit — should fill.
        let mark_at = Price::from_decimal(dec!(50100));
        let fill = sim.simulate_fill(&intent(OrderType::Limit, Side::Sell, Some(limit)), mark_at);
        assert!(fill.filled_qty > Decimal::ZERO);
    }
}
