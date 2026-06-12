//! Internal paper-execution surface (C-056).
//!
//! Layers, bottom to top — **no external venue calls anywhere**:
//! - four market-structure fill simulators (`clob`, `broker_quote`,
//!   `amm_swap`, `prediction`) plus a DEX paper wallet;
//! - [`policy`]: an ad hoc account model per asset class (cash / margin /
//!   binary, leverage, contract multiplier, starting cash);
//! - [`account`] + [`ledger`]: internal cash, positions, realized P&L, and an
//!   append-only transaction journal per asset class;
//! - [`engine`]: the [`PaperTradingEngine`] — mark-price board, order store
//!   with resting limit orders, one account per asset class;
//! - [`broker`] / [`account_source`]: `Broker` and `AccountSource`
//!   implementations backed entirely by the engine, so trading and account
//!   data never require venue credentials or network access.

pub mod account;
pub mod account_source;
pub mod amm_swap;
pub mod broker;
pub mod broker_quote;
pub mod clob;
pub mod engine;
pub mod ledger;
pub mod policy;
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

pub use account::{
    PaperAccount, PaperAccountSnapshot, PaperPosition, PaperPositionView, PaperTradeError,
};
pub use account_source::PaperAccountSource;
pub use amm_swap::AmmQuoteSwapSimulator;
pub use broker::PaperBroker;
pub use broker_quote::BrokerQuoteFillSimulator;
pub use clob::ClobFillSimulator;
pub use engine::{PaperOrderRecord, PaperTradingEngine, SimulatorSet};
pub use ledger::{PaperLedgerEntry, PaperLedgerKind};
pub use policy::{AccountKind, AccountPolicy, ALL_ASSET_CLASSES};
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
