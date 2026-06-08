//! Canonical lane name constants and typed `Lane` enum.
//!
//! Every event published on the NATS bus carries a `lane` field drawn from this
//! list.  Using typed values rather than bare strings catches typos at compile
//! time and makes exhaustive matching possible.

use serde::{Deserialize, Serialize};

pub const MARKET_TRADES: &str = "market.trades";
pub const MARKET_QUOTES: &str = "market.quotes";
pub const MARKET_ORDERBOOK_L2: &str = "market.orderbook.l2";
pub const MARKET_BARS_1S: &str = "market.bars.1s";
pub const MARKET_BARS_1M: &str = "market.bars.1m";
pub const MARKET_BARS_1M_REVISED: &str = "market.bars.1m.revised";
pub const FEATURES_TECHNICAL: &str = "features.technical";
pub const STRATEGY_SIGNALS: &str = "strategy.signals";
pub const ORDERS_COMMANDS: &str = "orders.commands";
pub const ORDERS_EVENTS: &str = "orders.events";
pub const POSITIONS_EVENTS: &str = "positions.events";
pub const QUARANTINE: &str = "quarantine";

/// Typed representation of a NATS lane.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    MarketTrades,
    MarketQuotes,
    MarketOrderbookL2,
    MarketBars1s,
    MarketBars1m,
    MarketBars1mRevised,
    FeaturesTechnical,
    StrategySignals,
    OrdersCommands,
    OrdersEvents,
    PositionsEvents,
    Quarantine,
}

impl Lane {
    /// Canonical string name used on the NATS subject.
    pub fn as_str(&self) -> &'static str {
        match self {
            Lane::MarketTrades => MARKET_TRADES,
            Lane::MarketQuotes => MARKET_QUOTES,
            Lane::MarketOrderbookL2 => MARKET_ORDERBOOK_L2,
            Lane::MarketBars1s => MARKET_BARS_1S,
            Lane::MarketBars1m => MARKET_BARS_1M,
            Lane::MarketBars1mRevised => MARKET_BARS_1M_REVISED,
            Lane::FeaturesTechnical => FEATURES_TECHNICAL,
            Lane::StrategySignals => STRATEGY_SIGNALS,
            Lane::OrdersCommands => ORDERS_COMMANDS,
            Lane::OrdersEvents => ORDERS_EVENTS,
            Lane::PositionsEvents => POSITIONS_EVENTS,
            Lane::Quarantine => QUARANTINE,
        }
    }

    /// Parse from a canonical string.  Returns `None` for unknown values.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            MARKET_TRADES => Some(Lane::MarketTrades),
            MARKET_QUOTES => Some(Lane::MarketQuotes),
            MARKET_ORDERBOOK_L2 => Some(Lane::MarketOrderbookL2),
            MARKET_BARS_1S => Some(Lane::MarketBars1s),
            MARKET_BARS_1M => Some(Lane::MarketBars1m),
            MARKET_BARS_1M_REVISED => Some(Lane::MarketBars1mRevised),
            FEATURES_TECHNICAL => Some(Lane::FeaturesTechnical),
            STRATEGY_SIGNALS => Some(Lane::StrategySignals),
            ORDERS_COMMANDS => Some(Lane::OrdersCommands),
            ORDERS_EVENTS => Some(Lane::OrdersEvents),
            POSITIONS_EVENTS => Some(Lane::PositionsEvents),
            QUARANTINE => Some(Lane::Quarantine),
            _ => None,
        }
    }
}

impl std::fmt::Display for Lane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lanes_round_trip() {
        let lanes = [
            Lane::MarketTrades,
            Lane::MarketQuotes,
            Lane::MarketOrderbookL2,
            Lane::MarketBars1s,
            Lane::MarketBars1m,
            Lane::MarketBars1mRevised,
            Lane::FeaturesTechnical,
            Lane::StrategySignals,
            Lane::OrdersCommands,
            Lane::OrdersEvents,
            Lane::PositionsEvents,
            Lane::Quarantine,
        ];
        for lane in &lanes {
            let s = lane.as_str();
            let back = Lane::from_str(s).expect("should parse back");
            assert_eq!(lane, &back, "failed for {s}");
        }
    }

    #[test]
    fn unknown_lane_returns_none() {
        assert!(Lane::from_str("not.a.lane").is_none());
    }
}
