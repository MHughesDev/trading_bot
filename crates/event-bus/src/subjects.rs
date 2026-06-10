//! Deterministic NATS subject naming for the browser NATS.ws feed (P2-T03).
//!
//! Pattern: `md.<data_type_key>.<venue_slug>.<asset_class_key>.<instrument_id>`
//!
//! Example: `md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD`

use domain::{AssetClass, DataType, SupportedVenue};

/// Build an OHLCV subject for browser NATS.ws subscription.
///
/// ```
/// use event_bus::subjects::ohlcv_subject;
/// use domain::{AssetClass, SupportedVenue};
/// let s = ohlcv_subject(SupportedVenue::Kraken, AssetClass::CryptoSpotCex, "BTC-USD");
/// assert_eq!(s, "md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD");
/// ```
pub fn ohlcv_subject(
    venue: SupportedVenue,
    asset_class: AssetClass,
    instrument_id: &str,
) -> String {
    format!(
        "md.{}.{}.{}.{}",
        DataType::MarketOhlcv.as_key(),
        venue.as_str(),
        asset_class_key(asset_class),
        instrument_id
    )
}

/// Build a subject for any data type (generic form).
///
/// ```
/// use event_bus::subjects::data_subject;
/// use domain::{AssetClass, DataType, SupportedVenue};
/// let s = data_subject(SupportedVenue::Kraken, AssetClass::CryptoSpotCex, DataType::MarketOhlcv, "BTC-USD");
/// assert_eq!(s, "md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD");
/// ```
pub fn data_subject(
    venue: SupportedVenue,
    asset_class: AssetClass,
    data_type: DataType,
    instrument_id: &str,
) -> String {
    format!(
        "md.{}.{}.{}.{}",
        data_type.as_key(),
        venue.as_str(),
        asset_class_key(asset_class),
        instrument_id
    )
}

/// Parse a subject back into its components.
///
/// Returns `None` if the subject does not match the `md.*.*.*.*` pattern.
pub fn parse_subject(subject: &str) -> Option<ParsedSubject<'_>> {
    let _parts: Vec<&str> = subject.splitn(5, '.').collect();
    // subject = md . <data_type_key> . <venue_slug> . <asset_class_key> . <instrument_id>
    // but data_type_key itself may contain dots (e.g. "market.ohlcv")
    // So we split differently: strip the "md." prefix then use known field widths.
    //
    // Format: md.<key1>.<key2>.<venue>.<asset_class>.<instrument_id>
    // where key1.key2 is the dotted DataType key.
    let stripped = subject.strip_prefix("md.")?;

    // Try to match each known DataType prefix greedily.
    let (dt_str, rest) = split_data_type(stripped)?;

    // rest = "<venue>.<asset_class>.<instrument_id>"
    let mut it = rest.splitn(3, '.');
    let venue_slug = it.next()?;
    let ac_slug = it.next()?;
    let instrument_id = it.next()?;

    Some(ParsedSubject {
        data_type_key: dt_str,
        venue_slug,
        asset_class_key: ac_slug,
        instrument_id,
    })
}

/// Components parsed from a NATS subject.
#[derive(Debug, PartialEq, Eq)]
pub struct ParsedSubject<'a> {
    pub data_type_key: &'a str,
    pub venue_slug: &'a str,
    pub asset_class_key: &'a str,
    pub instrument_id: &'a str,
}

fn split_data_type(s: &str) -> Option<(&str, &str)> {
    use domain::DataType;
    for dt in DataType::all() {
        let key = dt.as_key();
        if let Some(rest) = s.strip_prefix(key) {
            if let Some(after_dot) = rest.strip_prefix('.') {
                return Some((key, after_dot));
            }
        }
    }
    None
}

fn asset_class_key(ac: AssetClass) -> &'static str {
    match ac {
        AssetClass::CryptoSpotCex => "crypto_spot_cex",
        AssetClass::Equity => "equity",
        AssetClass::Etf => "etf",
        AssetClass::Bond => "bond",
        AssetClass::Nft => "nft",
        AssetClass::Fx => "fx",
        AssetClass::PredictionMarket => "prediction_market",
        AssetClass::Option => "option",
        AssetClass::CryptoSpotDex => "crypto_spot_dex",
        AssetClass::PerpetualSwap => "perpetual_swap",
        AssetClass::FuturesExpiring => "futures_expiring",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohlcv_subject_stable() {
        let s = ohlcv_subject(SupportedVenue::Kraken, AssetClass::CryptoSpotCex, "BTC-USD");
        assert_eq!(s, "md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD");
    }

    #[test]
    fn ohlcv_subject_round_trips() {
        let original = ohlcv_subject(SupportedVenue::Kraken, AssetClass::CryptoSpotCex, "BTC-USD");
        let parsed = parse_subject(&original).expect("should parse");
        assert_eq!(parsed.data_type_key, "market.ohlcv");
        assert_eq!(parsed.venue_slug, "kraken");
        assert_eq!(parsed.asset_class_key, "crypto_spot_cex");
        assert_eq!(parsed.instrument_id, "BTC-USD");
    }

    #[test]
    fn data_subject_with_funding_rate() {
        let s = data_subject(
            SupportedVenue::Kraken,
            AssetClass::PerpetualSwap,
            DataType::MarketFundingRate,
            "BTC-PERP",
        );
        assert_eq!(s, "md.market.funding_rate.kraken.perpetual_swap.BTC-PERP");
    }

    #[test]
    fn parse_invalid_subject_returns_none() {
        assert!(parse_subject("market.ohlcv.BTC-USD").is_none());
        assert!(parse_subject("md.unknown.key.kraken.BTC-USD").is_none());
    }
}
