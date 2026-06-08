//! Pure bar builder and order-book reconstruction.
//!
//! No I/O dependencies. Pure functions that accept events and return derived events.
//! The same code runs live (fed by the event bus) and in replay (fed by the archive).
//! This identity is the structural guarantee against lookahead bias.

pub mod bars;
pub mod orderbook;
pub mod watermark;

pub use bars::{timeframe_duration, window_start_for, BarBuilderConfig, BarEvent, BarState};
pub use orderbook::OrderBookBuilder;
pub use watermark::WatermarkPolicy;
