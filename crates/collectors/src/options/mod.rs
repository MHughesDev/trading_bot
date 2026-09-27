//! Options collectors.
//!
//! [`tradier`] is the pre-Set-P 1-minute OHLCV and quote collector that publishes
//! onto the event bus. [`tradier_chain_snapshot`] is the forward-stored chain
//! snapshot job of DA-12 / Set P P-0.5: a different shape entirely, because it
//! writes point-in-time rows into `option_chain_snapshots` with an observed
//! knowledge time and a declared 900 s vendor lag, rather than normalizing ticks.
//!
//! Both talk to Tradier, and they are **two different declared sources**:
//! `tradier_rest` (migration 0046, delayed quotes) and `tradier_chain_rest`
//! (migration 0098, chain snapshots). The distinction is what selects the right
//! declared lag for a row.

pub mod tradier;
pub mod tradier_chain_snapshot;
