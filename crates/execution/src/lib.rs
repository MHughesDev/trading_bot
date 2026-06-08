//! TODO(Phase 2): broker adapters, order state machine, fills, positions.
//! Three adapters: Coinbase (live), Alpaca (paper), market_simulator (backtest).
pub mod broker;
pub mod coinbase;
pub mod alpaca;
pub mod market_simulator;
pub mod order_state;
pub mod fills;
pub mod positions;
pub mod audit;
pub mod events;
