//! Apply-list compatibility filtering.
//!
//! A strategy is compatible with an instrument iff every lane in its
//! `CapabilityManifest.required_lanes` is in the instrument's provided set.
//! Incompatible strategies are **omitted** from apply-list responses, not flagged.

use domain::data_type::DataType;
use domain::instrument::AssetClass;

use crate::manifest::CapabilityManifest;

/// The data capabilities provided by a given instrument/venue combination.
pub struct InstrumentCapabilities {
    pub provided_lanes: Vec<DataType>,
}

impl InstrumentCapabilities {
    /// Derive capabilities from `AssetClass` using the platform default mapping.
    pub fn from_asset_class(ac: AssetClass) -> Self {
        Self {
            provided_lanes: default_provided_lanes(ac),
        }
    }
}

/// Default provided lanes per asset class.
///
/// All asset classes provide the minimum source baseline (`market.ohlcv`).
/// Additional lanes reflect what the venue's collector emits for that class.
pub fn default_provided_lanes(ac: AssetClass) -> Vec<DataType> {
    let mut lanes = vec![DataType::MarketOhlcv];
    match ac {
        AssetClass::CryptoSpotCex => {
            lanes.push(DataType::MarketTrade);
            lanes.push(DataType::MarketQuote);
        }
        AssetClass::Equity | AssetClass::Option | AssetClass::Fx => {
            lanes.push(DataType::MarketQuote);
        }
        AssetClass::PerpetualSwap | AssetClass::FuturesExpiring => {
            lanes.push(DataType::MarketTrade);
            lanes.push(DataType::MarketQuote);
            lanes.push(DataType::MarketFundingRate);
            lanes.push(DataType::MarketOpenInterest);
        }
        AssetClass::PredictionMarket => {
            lanes.push(DataType::PredictionMarketPrice);
        }
        AssetClass::CryptoSpotDex => {
            lanes.push(DataType::DexQuote);
        }
        AssetClass::Etf | AssetClass::Bond | AssetClass::Nft => {}
    }
    lanes
}

/// Returns `true` iff every lane required by `manifest` is in `caps.provided_lanes`.
pub fn is_compatible(manifest: &CapabilityManifest, caps: &InstrumentCapabilities) -> bool {
    manifest
        .required_lanes
        .iter()
        .all(|rl| caps.provided_lanes.contains(rl))
}
