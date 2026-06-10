//! Live venue broker adapters (C-059).
//!
//! Each adapter implements `Broker` for its venue's REST API.
//! All money fields use `Price`/`Size`. On a missing ack, query — never blind-retry.

pub mod kalshi;
pub mod oanda;
pub mod tradier;
pub mod tradovate;
pub mod zerox;

pub use kalshi::KalshiBroker;
pub use oanda::OandaBroker;
pub use tradier::TradierBroker;
pub use tradovate::TradovateBroker;
pub use zerox::ZeroXBroker;
