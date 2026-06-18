//! Core domain types — the irreversible foundation.
//!
//! No I/O.  No external side-effects.  All prices and sizes are newtypes over
//! `Decimal` with no `From<f64>` — the compiler enforces money safety.
//!
//! Re-exports the full public API so callers can write `use domain::Price`
//! without knowing the internal module layout.

// rkyv zero-copy impls on payload types require unsafe.  Every unsafe block
// carries an audit comment explaining why it is sound.
#![allow(unsafe_code)]

pub mod access;
pub mod data_type;
pub mod envelope;
pub mod error;
pub mod ids;
pub mod instrument;
pub mod interned;
pub mod lanes;
pub mod money;
pub mod order;
pub mod payloads;
pub mod position;
pub mod strategy_def;
pub mod timestamp;
pub mod trust;
pub mod venue;

// Convenience re-exports.
pub use access::{access_trusted, decode_from_bytes};
pub use data_type::{DataType, UnknownDataType};
pub use envelope::EventEnvelope;
pub use error::{NormalizeError, RiskRejection, ValidationError};
pub use ids::{event_id_from_key, onchain_key, sequenced_key, trade_key, DedupKey};
pub use instrument::{
    AssetClass, HaltPolicy, Instrument, InstrumentId, MarketStructure, SourceId, TradingSchedule,
    VenueId,
};
pub use interned::{
    epoch_hash, instrument_name, intern_instrument, intern_source, intern_venue, seed_intern_table,
    source_name, venue_name,
};
pub use lanes::{Lane, UnknownLane, QUARANTINE};
pub use money::{Price, Size};
pub use order::{Fill, OrderIntent, OrderRequest, OrderState, OrderType, Side, TimeInForce};
pub use payloads::{
    dex_quote::DexQuotePayload,
    funding_rate::FundingRatePayload,
    prediction_price::PredictionPricePayload,
    social_post::{InstrumentMention, SocialPostPayload},
    Payload,
};
pub use position::{Balance, Position};
pub use strategy_def::StrategyDefinition;
pub use timestamp::{compute_available_time, AvailableTimeParams, Timestamps};
pub use trust::TrustTier;
pub use venue::{SupportedVenue, UnknownVenue};

pub mod model;
pub mod model_def;

pub use model::{
    AliasName, CalibratedForecast, Direction, Forecast, ForecastRisk, ModelStatus, RiskAtLevel,
    RunStatus,
};
pub use model_def::{validate::validate as validate_model_def, ModelDefinition};
