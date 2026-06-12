//! Broker-quote paper fill simulator — used for equities, ETFs, options, bonds.
//!
//! Fills at a synthetic NBBO: `mark ± half_spread ± size_impact` for market
//! orders.  A linear size-impact model (identical in shape to the CLOB one)
//! penalises large orders that would visibly move the quoted price.
//!
//! Default configuration models a typical commission-free retail equity account:
//! - 3 bps half-spread (∼$0.06 on a $200 stock)
//! - 5 bps size impact at $1M notional, capped at 50 bps
//! - $0 commission (Schwab/Fidelity/Robinhood style)

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use domain::{
    money::Price,
    order::{OrderIntent, OrderType, Side, TimeInForce},
};

use super::{PaperFill, PaperFillSimulator};

/// Configurable half-spread and size impact for broker-quote fills.
#[derive(Debug, Clone)]
pub struct BrokerQuoteFillSimulator {
    /// Half-spread in basis points applied to the mark price.
    pub half_spread_bps: Decimal,
    /// Flat USD commission per fill (0 = commission-free).
    pub commission_flat: Decimal,
    /// Notional (quote currency) at which a market order pays
    /// `impact_bps_at_depth` of extra slippage.  `None` disables.
    pub depth_notional: Option<Decimal>,
    /// Extra slippage (bps) for a market order of exactly `depth_notional`.
    pub impact_bps_at_depth: Decimal,
    /// Upper bound on size impact (bps), however large the order.
    pub max_impact_bps: Decimal,
}

impl Default for BrokerQuoteFillSimulator {
    fn default() -> Self {
        Self {
            half_spread_bps: dec!(3),              // 0.03% — typical retail NBBO
            commission_flat: dec!(0),              // commission-free
            depth_notional: Some(dec!(1_000_000)), // $1M benchmark
            impact_bps_at_depth: dec!(5),          // 5 bps at $1M notional
            max_impact_bps: dec!(50),              // cap at 50 bps
        }
    }
}

impl BrokerQuoteFillSimulator {
    /// Extra slippage (bps) for a market order of `qty` at mark `m`.
    fn impact_bps(&self, m: Decimal, qty: Decimal) -> Decimal {
        let Some(depth) = self.depth_notional else {
            return Decimal::ZERO;
        };
        if depth <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        (self.impact_bps_at_depth * (qty * m) / depth).min(self.max_impact_bps)
    }
}

impl PaperFillSimulator for BrokerQuoteFillSimulator {
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill {
        let m = mark.inner();
        let spread_adj = m * self.half_spread_bps / dec!(10000);

        let fill_price = match intent.order_type {
            OrderType::Market => {
                let impact_adj = m * self.impact_bps(m, intent.size.inner()) / dec!(10000);
                let raw = match intent.side {
                    Side::Buy => m + spread_adj + impact_adj,
                    Side::Sell => m - spread_adj - impact_adj,
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
                        // Fill at mark with spread (price improvement — never worse than limit).
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
