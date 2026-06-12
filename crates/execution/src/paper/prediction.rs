//! Prediction-market paper fill simulator — used for Kalshi YES/NO binary contracts.
//!
//! Fills at the binary price in [0, 1] (representing a probability/payout).

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
}

impl Default for PredictionMarketFillSimulator {
    fn default() -> Self {
        Self {
            fee_coefficient: dec!(0.07), // Kalshi general schedule
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
        // Clamp mark to [0, 1] — binary price space.
        let p = mark.inner().clamp(Decimal::ZERO, Decimal::ONE);
        let fill_price = Price::from_decimal(p);

        let (qty, fills) = match intent.order_type {
            OrderType::Market => (intent.size.inner(), true),
            OrderType::Limit | OrderType::StopLimit => {
                if let Some(limit) = intent.limit_price {
                    let lp = limit.inner().clamp(Decimal::ZERO, Decimal::ONE);
                    let fills = match intent.side {
                        Side::Buy => p <= lp,
                        Side::Sell => p >= lp,
                    };
                    (
                        if fills {
                            intent.size.inner()
                        } else {
                            Decimal::ZERO
                        },
                        fills,
                    )
                } else {
                    (intent.size.inner(), true)
                }
            }
        };

        let fee = if fills && qty > Decimal::ZERO {
            self.fee(qty, fill_price.inner())
        } else {
            Decimal::ZERO
        };

        PaperFill {
            idempotency_key: intent.idempotency_key,
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_qty: if fills { qty } else { Decimal::ZERO },
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
        // 100 contracts at P=0.50: 0.07 × 100 × 0.25 = $1.75.
        let mid = sim.simulate_fill(&market(dec!(100)), Price::from_decimal(dec!(0.50)));
        assert_eq!(mid.fee, dec!(1.75));
        // Same size at P=0.95: 0.07 × 100 × 0.0475 = 0.3325 → $0.34 (round up).
        let edge = sim.simulate_fill(&market(dec!(100)), Price::from_decimal(dec!(0.95)));
        assert_eq!(edge.fee, dec!(0.34));
        assert!(edge.fee < mid.fee, "fees shrink toward the extremes");
    }
}
