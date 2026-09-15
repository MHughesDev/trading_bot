//! Domain-level error types.

use thiserror::Error;

/// Failure during raw-to-normalized event transformation.
///
/// When this error is produced, the raw bytes plus this error are published to
/// the `quarantine` lane — the failed event is never silently dropped.
#[derive(Clone, Debug, Error, serde::Serialize, serde::Deserialize)]
pub enum NormalizeError {
    #[error("price field '{field}' is not a valid decimal: {reason}")]
    InvalidPrice { field: String, reason: String },

    #[error("size field '{field}' is not a valid decimal: {reason}")]
    InvalidSize { field: String, reason: String },

    #[error("required field '{field}' is missing")]
    MissingField { field: String },

    #[error("unknown event type '{event_type}'")]
    UnknownEventType { event_type: String },

    #[error("schema version mismatch: expected '{expected}', got '{got}'")]
    SchemaMismatch { expected: String, got: String },

    #[error("deserialization failed: {0}")]
    Deserialize(String),
}

/// Failure during strategy-definition or order validation.
#[derive(Clone, Debug, Error, serde::Serialize, serde::Deserialize)]
pub enum ValidationError {
    #[error("strategy definition version '{version}' is not supported")]
    UnsupportedVersion { version: String },

    #[error("unknown node type '{node_type}' — fail-closed per spec")]
    UnknownNodeType { node_type: String },

    #[error("asset class mismatch: strategy requires '{required}', instrument is '{actual}'")]
    AssetClassMismatch { required: String, actual: String },

    #[error("risk override would loosen global limit for field '{field}'")]
    RiskOverrideTooPermissive { field: String },

    #[error("expression parse error in node '{node_id}': {reason}")]
    ExpressionParseError { node_id: String, reason: String },

    #[error("required field missing: {field}")]
    MissingField { field: String },
}

/// The risk gate refused to pass an order.
#[derive(Clone, Debug, Error, serde::Serialize, serde::Deserialize)]
pub enum RiskRejection {
    #[error("position limit exceeded for {instrument_id}: current {current}, requested {requested}, limit {limit}")]
    PositionLimitExceeded {
        instrument_id: String,
        current: String,
        requested: String,
        limit: String,
    },

    #[error("order rate limit exceeded: {orders_per_minute} orders/min, limit {limit}")]
    RateLimitExceeded { orders_per_minute: u32, limit: u32 },

    #[error("order rate limit exceeded (per second): {orders_last_second} orders/s, limit {limit_per_second}/s")]
    RateLimitPerSecondExceeded {
        orders_last_second: u32,
        limit_per_second: u32,
    },

    #[error("invalid tick size for {instrument_id}: price {price} is not a multiple of tick_size {tick_size}")]
    InvalidTickSize {
        instrument_id: String,
        price: String,
        tick_size: String,
    },

    #[error("kill switch active — all order flow halted")]
    KillSwitchActive,

    #[error("trading disabled globally")]
    TradingDisabled,

    #[error("instrument {instrument_id} is not active")]
    InstrumentInactive { instrument_id: String },

    #[error("price sanity check failed for {instrument_id}: limit_price {limit_price} is outside {band_bps}bps band around market {market_price}")]
    PriceSanityFailed {
        instrument_id: String,
        limit_price: String,
        market_price: String,
        band_bps: u32,
    },

    #[error("invalid lot size for {instrument_id}: size {size} is not a multiple of lot_size {lot_size}")]
    InvalidLotSize {
        instrument_id: String,
        size: String,
        lot_size: String,
    },

    #[error("daily loss limit exceeded: daily_loss {daily_loss_usd}, limit {limit_usd}")]
    DailyLossLimitExceeded {
        daily_loss_usd: String,
        limit_usd: String,
    },

    #[error("trust tier insufficient: required {required:?}, actual {actual:?}")]
    TrustTierInsufficient { required: String, actual: String },

    #[error("order rejected: {instrument_id} is outside its trading session")]
    OutsideTradingHours { instrument_id: String },

    #[error("order rejected: {instrument_id} is halted")]
    InstrumentHalted { instrument_id: String },

    // ── Fail-closed rejections: an input the gate needs was not measurable ────
    //
    // These exist because the alternative — substituting a permissive default
    // for an input we do not have — makes a limit that cannot fire report a
    // pass.  A limit that cannot fire must refuse, never default (SC-2, FM-2).
    /// No usable mark for the instrument, so exposure and price sanity cannot
    /// be judged.  Emitted instead of silently skipping those checks.
    #[error("order rejected: no current mark for {instrument_id} — risk context incomplete")]
    MarketDataUnavailable { instrument_id: String },

    /// Instrument metadata (tick size, lot size, session, halt policy) is not
    /// known, so lot/tick validity cannot be judged.
    #[error(
        "order rejected: no instrument metadata for {instrument_id} — lot/tick validity unknown"
    )]
    InstrumentMetadataUnavailable { instrument_id: String },

    /// Account state (equity, realized loss today) could not be read, so the
    /// daily-loss limit cannot fire.
    #[error("order rejected: account state unavailable ({detail}) — daily-loss limit cannot be evaluated")]
    AccountStateUnavailable { detail: String },

    /// The gate itself is not in a state where it can answer — a poisoned
    /// lock, or no risk-context provider installed in this process.  The gate
    /// is the one component that must always be able to say no; when it cannot
    /// reason, the answer is no.
    #[error("order rejected: risk gate unavailable ({detail})")]
    GateUnavailable { detail: String },
}
