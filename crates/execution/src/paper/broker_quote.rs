//! Broker-quote paper fill simulator — used for equities, options, FX.
//!
//! Fills at a synthetic NBBO: `mark ± half_spread`.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use domain::{
    money::Price,
    order::{OrderIntent, OrderType, Side, TimeInForce},
};

use super::{PaperFill, PaperFillSimulator};

/// Configurable half-spread for broker-quote fills.
#[derive(Debug, Clone)]
pub struct BrokerQuoteFillSimulator {
    /// Half-spread in basis points.
    pub half_spread_bps: Decimal,
    /// Commissions as a flat USD amount per fill.
    pub commission_flat: Decimal,
}

impl Default for BrokerQuoteFillSimulator {
    fn default() -> Self {
        Self {
            half_spread_bps: dec!(3), // 0.03%
            commission_flat: dec!(0), // commission-free (typical retail)
        }
    }
}

impl PaperFillSimulator for BrokerQuoteFillSimulator {
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill {
        let m = mark.inner();
        let spread_adj = m * self.half_spread_bps / dec!(10000);

        let fill_price = match intent.order_type {
            OrderType::Market => {
                let raw = match intent.side {
                    Side::Buy => m + spread_adj,
                    Side::Sell => m - spread_adj,
                };
                Price::from_decimal(raw.max(Decimal::ZERO))
            }
            OrderType::Limit | OrderType::StopLimit => {
                // Limit: fill at limit price if marketable, else respect TIF.
                if let Some(limit) = intent.limit_price {
                    let marketable = match intent.side {
                        // Buy limit is marketable when mark <= limit (we'd pay limit or better).
                        Side::Buy => mark.inner() <= limit.inner(),
                        // Sell limit is marketable when mark >= limit.
                        Side::Sell => mark.inner() >= limit.inner(),
                    };
                    if marketable {
                        // Fill at mark (price improvement for marketable limit).
                        let fill_raw = match intent.side {
                            Side::Buy => (m + spread_adj).min(limit.inner()),
                            Side::Sell => (m - spread_adj).max(limit.inner()),
                        };
                        Price::from_decimal(fill_raw.max(Decimal::ZERO))
                    } else {
                        // Non-marketable: IOC/FOK cancel immediately; GTC/Day rest.
                        match intent.time_in_force {
                            TimeInForce::Ioc | TimeInForce::Fok => {
                                return PaperFill {
                                    idempotency_key: intent.idempotency_key,
                                    instrument_id: intent.instrument_id.clone(),
                                    side: intent.side,
                                    filled_qty: Decimal::ZERO,
                                    fill_price: limit,
                                    fee: Decimal::ZERO,
                                };
                            }
                            TimeInForce::Gtc | TimeInForce::Day => {
                                // Resting order — no fill yet.
                                return PaperFill {
                                    idempotency_key: intent.idempotency_key,
                                    instrument_id: intent.instrument_id.clone(),
                                    side: intent.side,
                                    filled_qty: Decimal::ZERO,
                                    fill_price: limit,
                                    fee: Decimal::ZERO,
                                };
                            }
                        }
                    }
                } else {
                    let raw = match intent.side {
                        Side::Buy => m + spread_adj,
                        Side::Sell => m - spread_adj,
                    };
                    Price::from_decimal(raw.max(Decimal::ZERO))
                }
            }
        };

        PaperFill {
            idempotency_key: intent.idempotency_key,
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_qty: intent.size.inner(),
            fill_price,
            fee: self.commission_flat,
        }
    }
}
