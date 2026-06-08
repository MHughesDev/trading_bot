//! PURE bar builder and order-book reconstruction.
//!
//! No I/O dependencies. Pure functions that accept events and return derived events.
//! The same code runs live (fed by the event bus) and in replay (fed by the archive).
//! This identity is the structural guarantee against lookahead bias.
//!
//! TODO(Phase 1): implement bar builder, watermark, and orderbook reconstruction.
pub mod orderbook;
pub mod bars;
pub mod watermark;
