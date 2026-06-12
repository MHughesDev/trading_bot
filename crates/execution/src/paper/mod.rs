//! Internal paper-execution surface (C-056).
//!
//! One trait, four market-structure simulators, and a DEX paper wallet.
//! **No external venue calls** in any simulator — paper fills are always
//! computed locally.

pub mod amm_swap;
pub mod broker;
pub mod broker_quote;
pub mod clob;
pub mod prediction;
pub mod wallet;

use domain::{
    instrument::MarketStructure,
    money::Price,
    order::{OrderIntent, Side},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use amm_swap::AmmQuoteSwapSimulator;
pub use broker::PaperBroker;
pub use broker_quote::BrokerQuoteFillSimulator;
pub use clob::ClobFillSimulator;
pub use prediction::PredictionMarketFillSimulator;
pub use wallet::DexPaperWallet;

/// Result of a simulated fill — carries all information needed to write a
/// ledger event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperFill {
    pub idempotency_key: Uuid,
    pub instrument_id: String,
    pub side: Side,
    /// Quantity that was filled (may be less than ordered for partial fills).
    pub filled_qty: Decimal,
    /// Simulated fill price.
    pub fill_price: Price,
    /// Simulated exchange/spread fee (in quote currency).
    pub fee: Decimal,
}

/// Trait implemented by every paper fill simulator.
pub trait PaperFillSimulator: Send + Sync {
    /// Simulate executing `intent` at `mark` price.  Returns a `PaperFill`
    /// describing the simulated execution.  Never makes external calls.
    fn simulate_fill(&self, intent: &OrderIntent, mark: Price) -> PaperFill;
}

/// Select the correct simulator for a given market structure.
pub fn simulator_for(structure: MarketStructure) -> Box<dyn PaperFillSimulator> {
    match structure {
        MarketStructure::Clob => Box::new(ClobFillSimulator::default()),
        MarketStructure::BrokerQuote => Box::new(BrokerQuoteFillSimulator::default()),
        MarketStructure::AmmSwap => Box::new(AmmQuoteSwapSimulator::default()),
        MarketStructure::PredictionBinary => Box::new(PredictionMarketFillSimulator::default()),
    }
}
