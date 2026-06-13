//! `Instrument` metadata — makes asset classes a data concern, not a code concern.
//!
//! Components branch on *properties* of an `Instrument` (tick size, trading hours,
//! halt behavior) rather than on the specific `AssetClass` variant.  Adding a new
//! asset class to the system is therefore an additive data change, not a code change.

use chrono::Duration;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::trust::TrustTier;

/// Interned u32 handle for an instrument name (e.g. `"BTC-USD"`, `"AAPL"`).
///
/// Assigned at startup from the [`crate::interned::InternTable`].  Zero heap
/// allocation after interning — passes through ring buffers and NATS payloads
/// as a 4-byte integer.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq))]
pub struct InstrumentId(pub u32);

/// Interned u32 handle for a venue name (e.g. `"kraken"`, `"alpaca"`).
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq))]
pub struct VenueId(pub u32);

/// Interned u32 handle for a source name (e.g. `"kraken_ws"`, `"alpaca_ws"`).
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq))]
pub struct SourceId(pub u32);

/// Broad economic category of the instrument.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    CryptoSpotCex,
    Equity,
    Etf,
    CryptoSpotDex,
    FuturesExpiring,
    PerpetualSwap,
    Option,
    Bond,
    Fx,
    Nft,
    PredictionMarket,
}

/// All supported asset class identifiers in canonical snake_case form.
/// Shared between the API `/api/assets` route and the MCP `list_instruments` tool
/// so they can never diverge.
pub const ALL_ASSET_CLASSES: &[&str] = &[
    "crypto_spot_cex",
    "equity",
    "etf",
    "crypto_spot_dex",
    "futures_expiring",
    "perpetual_swap",
    "option",
    "bond",
    "fx",
    "nft",
    "prediction_market",
];

impl AssetClass {
    /// Canonical snake_case string key (matches serde `rename_all = "snake_case"`).
    ///
    /// Returns a `&'static str` — zero allocation, safe to use in hot paths
    /// and in places that previously called `serde_json::to_value(a).as_str()`.
    pub fn as_str(self) -> &'static str {
        match self {
            AssetClass::CryptoSpotCex => "crypto_spot_cex",
            AssetClass::Equity => "equity",
            AssetClass::Etf => "etf",
            AssetClass::CryptoSpotDex => "crypto_spot_dex",
            AssetClass::FuturesExpiring => "futures_expiring",
            AssetClass::PerpetualSwap => "perpetual_swap",
            AssetClass::Option => "option",
            AssetClass::Bond => "bond",
            AssetClass::Fx => "fx",
            AssetClass::Nft => "nft",
            AssetClass::PredictionMarket => "prediction_market",
        }
    }

    /// Market microstructure model used for paper execution simulation.
    pub fn market_structure(self) -> MarketStructure {
        match self {
            AssetClass::CryptoSpotCex
            | AssetClass::Fx
            | AssetClass::Etf
            | AssetClass::Bond
            | AssetClass::FuturesExpiring
            | AssetClass::PerpetualSwap => MarketStructure::Clob,
            AssetClass::Equity | AssetClass::Option => MarketStructure::BrokerQuote,
            AssetClass::CryptoSpotDex | AssetClass::Nft => MarketStructure::AmmSwap,
            AssetClass::PredictionMarket => MarketStructure::PredictionBinary,
        }
    }
}

/// Market microstructure model — determines which paper fill simulator to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketStructure {
    /// Central limit order book (most exchange-traded instruments).
    Clob,
    /// Dealer/broker quote — fill at quoted mid ± spread.
    BrokerQuote,
    /// AMM or DEX swap — fill at on-chain pool price.
    AmmSwap,
    /// Binary prediction market — fill at YES/NO price.
    PredictionBinary,
}

/// When the instrument trades.  Empty `sessions` list means 24/7.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingSession {
    /// e.g. `"09:30"` in `timezone`.
    pub open: String,
    /// e.g. `"16:00"` in `timezone`.
    pub close: String,
}

/// Full trading schedule for an instrument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingSchedule {
    /// IANA timezone string (e.g. `"America/New_York"`, `"UTC"`).
    pub timezone: String,
    /// Regular sessions.  Empty = 24/7 (typical for crypto).
    pub sessions: Vec<TradingSession>,
    pub has_pre_market: bool,
    pub has_post_market: bool,
}

impl TradingSchedule {
    /// Convenience constructor for a 24/7 instrument (crypto spot).
    pub fn always_open() -> Self {
        Self {
            timezone: "UTC".into(),
            sessions: vec![],
            has_pre_market: false,
            has_post_market: false,
        }
    }

    pub fn is_24_7(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// Policy for handling exchange halts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HaltPolicy {
    /// Exchange can halt trading on this instrument.
    Haltable,
    /// Instrument never halts (typical for permissionless crypto).
    NonHaltable,
}

/// Complete instrument metadata record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instrument {
    /// Human-readable instrument symbol, e.g. `"BTC-USD"`, `"AAPL"`.
    pub instrument_id: String,
    pub asset_class: AssetClass,
    /// Human-readable venue name, e.g. `"kraken"`, `"alpaca"`.
    pub venue_id: String,
    /// Decimal places for the base asset quantity.
    pub base_precision: u32,
    /// Decimal places for the quote asset price.
    pub quote_precision: u32,
    /// Minimum price movement; must be a `Decimal` (never a float).
    pub tick_size: Decimal,
    /// Minimum order quantity; must be a `Decimal` (never a float).
    pub lot_size: Decimal,
    pub trading_hours: TradingSchedule,
    pub halt_behavior: HaltPolicy,
    pub trust_tier: TrustTier,
    pub active: bool,
    /// Default watermark duration for this source (used by `available_time` calc).
    /// Defaults to 2 seconds for liquid CEX streams.
    #[serde(default = "default_watermark_secs")]
    pub watermark_secs: i64,
}

fn default_watermark_secs() -> i64 {
    2
}

impl Instrument {
    /// Watermark as a `chrono::Duration`.
    pub fn watermark(&self) -> Duration {
        Duration::seconds(self.watermark_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn crypto_24_7_constructs() {
        let inst = Instrument {
            instrument_id: "BTC-USDT".into(),
            asset_class: AssetClass::CryptoSpotCex,
            venue_id: "coinbase".into(),
            base_precision: 8,
            quote_precision: 2,
            tick_size: dec!(0.01),
            lot_size: dec!(0.00001),
            trading_hours: TradingSchedule::always_open(),
            halt_behavior: HaltPolicy::NonHaltable,
            trust_tier: TrustTier::CentralizedExchange,
            active: true,
            watermark_secs: 2,
        };
        assert!(inst.trading_hours.is_24_7());
    }

    #[test]
    fn equity_with_session_constructs() {
        let inst = Instrument {
            instrument_id: "AAPL".into(),
            asset_class: AssetClass::Equity,
            venue_id: "alpaca".into(),
            base_precision: 2,
            quote_precision: 2,
            tick_size: dec!(0.01),
            lot_size: dec!(1),
            trading_hours: TradingSchedule {
                timezone: "America/New_York".into(),
                sessions: vec![TradingSession {
                    open: "09:30".into(),
                    close: "16:00".into(),
                }],
                has_pre_market: true,
                has_post_market: true,
            },
            halt_behavior: HaltPolicy::Haltable,
            trust_tier: TrustTier::Regulated,
            active: true,
            watermark_secs: 2,
        };
        assert!(!inst.trading_hours.is_24_7());
    }

    #[test]
    fn serde_round_trip() {
        let inst = Instrument {
            instrument_id: "ETH-USDT".into(),
            asset_class: AssetClass::CryptoSpotCex,
            venue_id: "kraken".into(),
            base_precision: 8,
            quote_precision: 2,
            tick_size: dec!(0.01),
            lot_size: dec!(0.001),
            trading_hours: TradingSchedule::always_open(),
            halt_behavior: HaltPolicy::NonHaltable,
            trust_tier: TrustTier::CentralizedExchange,
            active: true,
            watermark_secs: 2,
        };
        let json = serde_json::to_string(&inst).unwrap();
        let back: Instrument = serde_json::from_str(&json).unwrap();
        assert_eq!(inst.instrument_id, back.instrument_id);
        assert_eq!(inst.tick_size, back.tick_size);
    }
}
