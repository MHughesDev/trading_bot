//! TODO(Phase 1): resolves (AssetClass, DataType, Instrument) → VenueId and
//! starts/stops collectors on demand. Never starts a collector at system init.
//! Routing: Crypto → Kraken (data); Equity → Alpaca data feed.
//! Execution routing is separate: live→Coinbase, paper→Alpaca, backtest→market_simulator.
pub mod lifecycle;
pub mod registry;
pub mod resolver;
