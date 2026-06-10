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
    /// Trading fee as a fraction of notional (e.g. 0.007 = 0.7%).
    pub fee_rate: Decimal,
}

impl Default for PredictionMarketFillSimulator {
    fn default() -> Self {
        Self {
            fee_rate: dec!(0.007), // Kalshi typical
        }
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

        let fee = qty * fill_price.inner() * self.fee_rate;

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
