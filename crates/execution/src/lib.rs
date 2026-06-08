//! TODO(Phase 2): broker adapters, order state machine, fills, positions.
//! Three adapters: Coinbase (live), Alpaca (paper), market_simulator (backtest).
pub mod alpaca;
pub mod audit;
pub mod broker;
pub mod coinbase;
pub mod events;
pub mod fills;
pub mod market_simulator;
pub mod order_state;
pub mod positions;
