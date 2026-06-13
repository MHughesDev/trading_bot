//! Canonical lane name constants and typed `Lane` enum.
//!
//! Every event published on the NATS bus carries a `lane` field drawn from this
//! list.  Using typed values rather than bare strings catches typos at compile
//! time and makes exhaustive matching possible.

use std::fmt;
use std::str::FromStr;

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
/// Virtual UI-only lane exposed via the WebSocket gateway.
pub const UI_ORDERBOOK_SNAPSHOT: &str = "ui.orderbook.snapshot";

/// All public data lanes visible via `/api/streams/available` and MCP `list_lanes`.
pub const ALL_LANES: &[&str] = &[
    MARKET_TRADES,
    MARKET_QUOTES,
    MARKET_ORDERBOOK_L2,
    MARKET_BARS_1S,
    MARKET_BARS_1M,
    MARKET_BARS_1M_REVISED,
    FEATURES_TECHNICAL,
    STRATEGY_SIGNALS,
    ORDERS_COMMANDS,
    ORDERS_EVENTS,
    POSITIONS_EVENTS,
    UI_ORDERBOOK_SNAPSHOT,
];

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
}

/// Error returned when parsing an unknown lane string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownLane(pub String);

impl fmt::Display for UnknownLane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown lane: {:?}", self.0)
    }
}

impl std::error::Error for UnknownLane {}

impl FromStr for Lane {
    type Err = UnknownLane;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            MARKET_TRADES => Ok(Lane::MarketTrades),
            MARKET_QUOTES => Ok(Lane::MarketQuotes),
            MARKET_ORDERBOOK_L2 => Ok(Lane::MarketOrderbookL2),
            MARKET_BARS_1S => Ok(Lane::MarketBars1s),
            MARKET_BARS_1M => Ok(Lane::MarketBars1m),
            MARKET_BARS_1M_REVISED => Ok(Lane::MarketBars1mRevised),
            FEATURES_TECHNICAL => Ok(Lane::FeaturesTechnical),
            STRATEGY_SIGNALS => Ok(Lane::StrategySignals),
            ORDERS_COMMANDS => Ok(Lane::OrdersCommands),
            ORDERS_EVENTS => Ok(Lane::OrdersEvents),
            POSITIONS_EVENTS => Ok(Lane::PositionsEvents),
            QUARANTINE => Ok(Lane::Quarantine),
            other => Err(UnknownLane(other.to_owned())),
        }
    }
}

impl fmt::Display for Lane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            let back: Lane = s.parse().expect("should parse back");
            assert_eq!(lane, &back, "failed for {s}");
        }
    }

    #[test]
    fn unknown_lane_returns_err() {
        assert!("not.a.lane".parse::<Lane>().is_err());
    }
}
