//! `DataType` — single source-of-truth for every data primitive and derived
//! capability in the platform.  Used as demand keys, manifest entries, and
//! collector capability declarations.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Every data primitive and derived capability the platform knows about.
///
/// Serialises to/from a dotted string key (e.g. `"market.ohlcv"`).
/// The minimum source-data baseline is [`DataType::MarketOhlcv`]; order-book,
/// DOM, and tick-list types are explicitly absent (invariant C-129).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum DataType {
    /// 1-minute OHLCV bar — the minimum source baseline.
    MarketOhlcv,
    /// Individual trade tick.
    MarketTrade,
    /// NBBO or top-of-book quote.
    MarketQuote,
    /// Perpetual-swap funding rate.
    MarketFundingRate,
    /// Futures/perp open interest.
    MarketOpenInterest,
    /// Prediction-market contract price (e.g. Kalshi YES/NO).
    PredictionMarketPrice,
    /// DEX/AMM firm quote snapshot.
    DexQuote,
    /// Social-media post (e.g. Reddit).
    SocialPost,
    /// Full web-page HTML snapshot.
    WebPageSnapshot,
    /// Parsed news article.
    NewsArticle,
}

impl DataType {
    /// Canonical dotted string key used as a demand key and manifest entry.
    pub fn as_key(self) -> &'static str {
        match self {
            Self::MarketOhlcv => "market.ohlcv",
            Self::MarketTrade => "market.trade",
            Self::MarketQuote => "market.quote",
            Self::MarketFundingRate => "market.funding_rate",
            Self::MarketOpenInterest => "market.open_interest",
            Self::PredictionMarketPrice => "prediction.price",
            Self::DexQuote => "dex.quote",
            Self::SocialPost => "social.post",
            Self::WebPageSnapshot => "web.page_snapshot",
            Self::NewsArticle => "news.article",
        }
    }

    /// All variants — used for seeding the `data_type_registry` table.
    pub fn all() -> &'static [DataType] {
        &[
            Self::MarketOhlcv,
            Self::MarketTrade,
            Self::MarketQuote,
            Self::MarketFundingRate,
            Self::MarketOpenInterest,
            Self::PredictionMarketPrice,
            Self::DexQuote,
            Self::SocialPost,
            Self::WebPageSnapshot,
            Self::NewsArticle,
        ]
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_key())
    }
}

impl FromStr for DataType {
    type Err = UnknownDataType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "market.ohlcv" => Ok(Self::MarketOhlcv),
            "market.trade" => Ok(Self::MarketTrade),
            "market.quote" => Ok(Self::MarketQuote),
            "market.funding_rate" => Ok(Self::MarketFundingRate),
            "market.open_interest" => Ok(Self::MarketOpenInterest),
            "prediction.price" => Ok(Self::PredictionMarketPrice),
            "dex.quote" => Ok(Self::DexQuote),
            "social.post" => Ok(Self::SocialPost),
            "web.page_snapshot" => Ok(Self::WebPageSnapshot),
            "news.article" => Ok(Self::NewsArticle),
            other => Err(UnknownDataType(other.to_owned())),
        }
    }
}

/// Error returned when a dotted key does not map to any known `DataType`.
#[derive(Debug, thiserror::Error)]
#[error("unknown data type key: {0}")]
pub struct UnknownDataType(pub String);

// Serde support via String conversion.
impl From<DataType> for String {
    fn from(dt: DataType) -> Self {
        dt.as_key().to_owned()
    }
}

impl TryFrom<String> for DataType {
    type Error = UnknownDataType;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_round_trip_via_as_key_and_parse() {
        for &dt in DataType::all() {
            let key = dt.as_key();
            let parsed: DataType = key.parse().expect("round-trip failed");
            assert_eq!(dt, parsed, "round-trip failed for {key}");
        }
    }

    #[test]
    fn market_ohlcv_key_is_correct() {
        assert_eq!(DataType::MarketOhlcv.as_key(), "market.ohlcv");
    }

    #[test]
    fn dex_quote_parses_from_string() {
        let dt: DataType = "dex.quote".parse().unwrap();
        assert_eq!(dt, DataType::DexQuote);
    }

    #[test]
    fn unknown_key_returns_error() {
        let result = "order.book".parse::<DataType>();
        assert!(result.is_err());
    }

    #[test]
    fn serde_round_trip() {
        let dt = DataType::MarketFundingRate;
        let json = serde_json::to_string(&dt).unwrap();
        assert_eq!(json, r#""market.funding_rate""#);
        let back: DataType = serde_json::from_str(&json).unwrap();
        assert_eq!(dt, back);
    }
}
