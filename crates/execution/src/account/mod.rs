//! Per-venue `AccountSource` REST adapters (C-017/C-092).
//!
//! Fire on-demand when the user navigates to Dashboard — no polling.
//! Credentials are decrypted by the credential service before being passed here.

pub mod alpaca;
pub mod coinbase;
pub mod kalshi;
pub mod kraken;
pub mod oanda;
pub mod tradier;
pub mod tradovate;

pub use alpaca::AlpacaAccountSource;
pub use coinbase::CoinbaseAccountSource;
pub use kalshi::KalshiAccountSource;
pub use kraken::KrakenAccountSource;
pub use oanda::OandaAccountSource;
pub use tradier::TradierAccountSource;
pub use tradovate::TradovateAccountSource;
