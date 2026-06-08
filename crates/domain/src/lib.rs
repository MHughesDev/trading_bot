//! Core domain types — the irreversible foundation.
//!
//! No I/O.  No external side-effects.  All prices and sizes are newtypes over
//! `Decimal` with no `From<f64>` — the compiler enforces money safety.
//!
//! Re-exports the full public API so callers can write `use domain::Price`
//! without knowing the internal module layout.

pub mod envelope;
pub mod error;
pub mod ids;
pub mod instrument;
pub mod lanes;
pub mod money;
pub mod order;
pub mod payloads;
pub mod position;
pub mod strategy_def;
pub mod timestamp;
pub mod trust;

// Convenience re-exports.
pub use envelope::EventEnvelope;
pub use error::{NormalizeError, RiskRejection, ValidationError};
pub use ids::{event_id_from_key, onchain_key, sequenced_key, trade_key, DedupKey};
pub use instrument::{AssetClass, HaltPolicy, Instrument, InstrumentId, TradingSchedule, VenueId};
pub use lanes::{Lane, QUARANTINE};
pub use money::{Price, Size};
pub use order::{Fill, OrderIntent, OrderRequest, OrderState, OrderType, Side};
pub use payloads::Payload;
pub use position::{Balance, Position};
pub use strategy_def::StrategyDefinition;
pub use timestamp::{compute_available_time, AvailableTimeParams, Timestamps};
pub use trust::TrustTier;
