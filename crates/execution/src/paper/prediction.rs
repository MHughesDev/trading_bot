//! Prediction-market paper fill simulator — used for Kalshi YES/NO binary contracts.
//!
//! Fills at the binary price in [0, 1] (representing a probability/payout).
//! Market orders pay a half-spread of 1 cent — Kalshi's typical bid-ask on
//! liquid contracts.  Limit orders fill at the mark price (no spread).

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use domain::{
    money::Price,
    order::{OrderIntent, OrderType, Side},
};

use super::{PaperFill, PaperFillSimulator};

/// Prediction-market binary fill simulator.
#[derive(Debug, Clone)]
pub struct PredictionMarketFillSimulator {
    /// Fee coefficient in Kalshi's schedule:
    /// `fee = coefficient × contracts × P × (1 − P)`, rounded up to the cent.
    /// Maximal at P = 0.5, vanishing toward 0 / 1 — unlike a flat rate, which
    /// overcharges mid-range prices and undercharges the extremes.
    pub fee_coefficient: Decimal,
    /// Half-spread in dollar terms applied to market orders.
    /// Kalshi contracts trade in $0.01 increments; a 1-cent half-spread models
    /// a 2-cent-wide bid-ask, typical for liquid contracts.
    pub half_spread_cents: Decimal,
}

impl Default for PredictionMarketFillSimulator {
    fn default() -> Self {
        Self {
            fee_coefficient: dec!(0.07),  // Kalshi general schedule
            half_spread_cents: dec!(0.01), // 1-cent half-spread = 2-cent-wide market
        }
    }
}

impl PredictionMarketFillSimulator {
    /// Trading fee for `qty` contracts at binary price `p`, rounded up to the
    /// cent (Kalshi rounds fees up).
    fn fee(&self, qty: Decimal, p: Decimal) -> Decimal {
        let raw = self.fee_coefficient * qty * p * (Decimal::ONE - p);
        (raw * dec!(100)).ceil() / dec!(100)
    }
}

impl PaperFillSimulator for PredictionMarketFillSimulator {
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill {
        let p = mark.inner().clamp(Decimal::ZERO, Decimal::ONE);

        let (fill_price, qty) = match intent.order_type {
            OrderType::Market => {
                // Market orders cross the spread.
                let fill_raw = match intent.side {
                    Side::Buy => p + self.half_spread_cents,
                    Side::Sell => p - self.half_spread_cents,
                };
                (
                    Price::from_decimal(fill_raw.clamp(Decimal::ZERO, Decimal::ONE)),
                    intent.size.inner(),
                )
            }
            OrderType::Limit | OrderType::StopLimit => {
                if let Some(limit) = intent.limit_price {
                    let lp = limit.inner().clamp(Decimal::ZERO, Decimal::ONE);
                    // Limit fills at mark (no spread) when marketable.
                    let fills = match intent.side {
                        Side::Buy => p <= lp,
                        Side::Sell => p >= lp,
                    };
                    (
                        Price::from_decimal(p),
                        if fills { intent.size.inner() } else { Decimal::ZERO },
                    )
                } else {
                    // No limit price — treat as market.
                    let fill_raw = match intent.side {
                        Side::Buy => p + self.half_spread_cents,
                        Side::Sell => p - self.half_spread_cents,
                    };
                    (
                        Price::from_decimal(fill_raw.clamp(Decimal::ZERO, Decimal::ONE)),
                        intent.size.inner(),
                    )
                }
            }
        };

        let fee = if qty > Decimal::ZERO {
            self.fee(qty, fill_price.inner())
        } else {
            Decimal::ZERO
        };

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

    fn market(qty: Decimal) -> OrderIntent {
        OrderIntent::new(
            "KX-EVENT",
            Side::Buy,
            OrderType::Market,
            Size::from_decimal(qty),
            None,
            None,
        )
    }

    #[test]
    fn fee_follows_kalshi_p_one_minus_p_schedule() {
        let sim = PredictionMarketFillSimulator::default();
        // Market buy at P=0.50 fills at 0.51 (half_spread = $0.01).
        // Fee: 0.07 × 100 × 0.51 × 0.49 = 1.7493 → $1.75 (round up).
        let mid = sim.simulate_fill(&market(dec!(100)), Price::from_decimal(dec!(0.50)));
        assert_eq!(mid.fee, dec!(1.75));
        // Market buy at P=0.95 fills at 0.96.
        // Fee: 0.07 × 100 × 0.96 × 0.04 = 0.2688 → $0.27 (round up).
        let edge = sim.simulate_fill(&market(dec!(100)), Price::from_decimal(dec!(0.95)));
        assert_eq!(edge.fee, dec!(0.27));
        assert!(edge.fee < mid.fee, "fees shrink toward the extremes");
    }

    #[test]
    fn market_order_fill_price_crosses_spread() {
        let sim = PredictionMarketFillSimulator::default();
        let buy = sim.simulate_fill(&market(dec!(10)), Price::from_decimal(dec!(0.60)));
        assert_eq!(buy.fill_price.inner(), dec!(0.61)); // buys lift the ask

        let mut sell_intent = market(dec!(10));
        sell_intent.side = Side::Sell;
        let sell = sim.simulate_fill(&sell_intent, Price::from_decimal(dec!(0.60)));
        assert_eq!(sell.fill_price.inner(), dec!(0.59)); // sells hit the bid
    }

    #[test]
    fn fill_price_clamped_at_boundaries() {
        let sim = PredictionMarketFillSimulator::default();
        // Buy at P=1.00 — fill price clamped to 1.00 (can't exceed par).
        let at_par = sim.simulate_fill(&market(dec!(10)), Price::from_decimal(dec!(1.00)));
        assert_eq!(at_par.fill_price.inner(), Decimal::ONE);

        // Sell at P=0.00 — fill price clamped to 0.00.
        let mut sell_zero = market(dec!(10));
        sell_zero.side = Side::Sell;
        let at_zero = sim.simulate_fill(&sell_zero, Price::from_decimal(dec!(0.00)));
        assert_eq!(at_zero.fill_price.inner(), Decimal::ZERO);
    }
}
