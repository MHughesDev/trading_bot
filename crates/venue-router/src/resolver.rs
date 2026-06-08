//! Maps (AssetClass, Lane) to a venue identifier.

use domain::{AssetClass, Lane};

/// Stateless venue resolver.
pub struct Resolver;

impl Resolver {
    /// Map an `(AssetClass, Lane)` pair to a venue_id string.
    ///
    /// * Crypto (spot or derivative) → `"kraken"`
    /// * US equity → `"alpaca"`
    /// * Everything else → `"unknown"`
    pub fn venue_id_for(asset_class: &AssetClass, _lane: &Lane) -> &'static str {
        match asset_class {
            AssetClass::CryptoSpotCex | AssetClass::CryptoSpotDex | AssetClass::PerpetualSwap => {
                "kraken"
            }
            AssetClass::Equity | AssetClass::Etf => "alpaca",
            _ => "unknown",
        }
    }
}
