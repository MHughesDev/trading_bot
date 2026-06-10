//! `SupportedVenue` — typed enum of every integration venue with capability
//! metadata.  Used for credential storage, collector routing, and capability
//! manifests.

use crate::instrument::AssetClass;

/// Every venue the platform integrates with (C-055).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SupportedVenue {
    /// Kraken — crypto market data (WS feed).
    Kraken,
    /// Coinbase Advanced Trade — crypto live execution.
    Coinbase,
    /// Alpaca — equity market data + execution.
    Alpaca,
    /// OANDA — FX (demo MVP).
    Oanda,
    /// Kalshi — prediction markets + perpetual swaps.
    Kalshi,
    /// Tradier — options.
    Tradier,
    /// 0x — DEX swap aggregation.
    ZeroX,
    /// Tradovate — futures (demo first).
    Tradovate,
}

impl SupportedVenue {
    /// Stable lowercase slug used in credential storage and NATS subjects.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kraken => "kraken",
            Self::Coinbase => "coinbase",
            Self::Alpaca => "alpaca",
            Self::Oanda => "oanda",
            Self::Kalshi => "kalshi",
            Self::Tradier => "tradier",
            Self::ZeroX => "zerox",
            Self::Tradovate => "tradovate",
        }
    }

    /// Whether this venue provides market-data collection.
    pub fn provides_data(self) -> bool {
        matches!(
            self,
            Self::Kraken
                | Self::Alpaca
                | Self::Oanda
                | Self::Kalshi
                | Self::Tradier
                | Self::ZeroX
                | Self::Tradovate
        )
    }

    /// Whether this venue provides live order execution.
    pub fn provides_execution(self) -> bool {
        matches!(
            self,
            Self::Coinbase
                | Self::Alpaca
                | Self::Oanda
                | Self::Kalshi
                | Self::Tradier
                | Self::ZeroX
                | Self::Tradovate
        )
    }

    /// Asset classes supported by this venue.
    pub fn supported_asset_classes(self) -> &'static [AssetClass] {
        match self {
            Self::Kraken => &[AssetClass::CryptoSpotCex, AssetClass::PerpetualSwap],
            Self::Coinbase => &[AssetClass::CryptoSpotCex],
            Self::Alpaca => &[AssetClass::Equity],
            Self::Oanda => &[AssetClass::Fx],
            Self::Kalshi => &[AssetClass::PredictionMarket, AssetClass::PerpetualSwap],
            Self::Tradier => &[AssetClass::Option],
            Self::ZeroX => &[AssetClass::CryptoSpotDex],
            Self::Tradovate => &[AssetClass::FuturesExpiring],
        }
    }

    /// All venue variants — useful for iteration.
    pub fn all() -> &'static [SupportedVenue] {
        &[
            Self::Kraken,
            Self::Coinbase,
            Self::Alpaca,
            Self::Oanda,
            Self::Kalshi,
            Self::Tradier,
            Self::ZeroX,
            Self::Tradovate,
        ]
    }
}

impl std::fmt::Display for SupportedVenue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SupportedVenue {
    type Err = UnknownVenue;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "kraken" => Ok(Self::Kraken),
            "coinbase" => Ok(Self::Coinbase),
            "alpaca" => Ok(Self::Alpaca),
            "oanda" => Ok(Self::Oanda),
            "kalshi" => Ok(Self::Kalshi),
            "tradier" => Ok(Self::Tradier),
            "zerox" => Ok(Self::ZeroX),
            "tradovate" => Ok(Self::Tradovate),
            other => Err(UnknownVenue(other.to_owned())),
        }
    }
}

/// Error returned when a string does not map to a known venue slug.
#[derive(Debug, thiserror::Error)]
#[error("unknown venue slug: {0}")]
pub struct UnknownVenue(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kalshi_supports_prediction_market_and_perpetual_swap() {
        let classes = SupportedVenue::Kalshi.supported_asset_classes();
        assert!(classes.contains(&AssetClass::PredictionMarket));
        assert!(classes.contains(&AssetClass::PerpetualSwap));
    }

    #[test]
    fn zerox_provides_execution_and_slug_round_trips() {
        assert!(SupportedVenue::ZeroX.provides_execution());
        let slug = SupportedVenue::ZeroX.as_str();
        let parsed: SupportedVenue = slug.parse().unwrap();
        assert_eq!(parsed, SupportedVenue::ZeroX);
    }

    #[test]
    fn all_venues_slug_round_trip() {
        for &v in SupportedVenue::all() {
            let slug = v.as_str();
            let parsed: SupportedVenue = slug
                .parse()
                .unwrap_or_else(|_| panic!("parse failed for {slug}"));
            assert_eq!(v, parsed);
        }
    }

    #[test]
    fn unknown_slug_returns_error() {
        let result = "bitfinex".parse::<SupportedVenue>();
        assert!(result.is_err());
    }
}
