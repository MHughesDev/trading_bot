//! `Instrument` metadata — makes asset classes a data concern, not a code concern.
//!
//! Components branch on *properties* of an `Instrument` (tick size, trading hours,
//! halt behavior) rather than on the specific `AssetClass` variant.  Adding a new
//! asset class to the system is therefore an additive data change, not a code change.

use chrono::Duration;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::trust::TrustTier;

/// Unique identifier for an instrument (e.g. `"BTC-USDT"`, `"AAPL"`).
pub type InstrumentId = String;

/// Venue/broker identifier (e.g. `"coinbase"`, `"alpaca"`, `"kraken"`).
pub type VenueId = String;

/// Broad economic category of the instrument.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    pub instrument_id: InstrumentId,
    pub asset_class: AssetClass,
    pub venue_id: VenueId,
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
