//! P2-T03 acceptance tests: ohlcv_subject is stable and round-trips.

use domain::{AssetClass, DataType, SupportedVenue};
use event_bus::subjects::{data_subject, ohlcv_subject, parse_subject};

#[test]
fn ohlcv_subject_is_stable() {
    let s = ohlcv_subject(SupportedVenue::Kraken, AssetClass::CryptoSpotCex, "BTC-USD");
    assert_eq!(s, "md.market.ohlcv.kraken.crypto_spot_cex.BTC-USD");
}

#[test]
fn ohlcv_subject_round_trips() {
    let subject = ohlcv_subject(SupportedVenue::Alpaca, AssetClass::Equity, "AAPL");
    let parsed = parse_subject(&subject).expect("should parse");
    assert_eq!(parsed.data_type_key, "market.ohlcv");
    assert_eq!(parsed.venue_slug, "alpaca");
    assert_eq!(parsed.asset_class_key, "equity");
    assert_eq!(parsed.instrument_id, "AAPL");
}

#[test]
fn data_subject_social_post_round_trips() {
    let subject = data_subject(
        SupportedVenue::Kraken,
        AssetClass::CryptoSpotCex,
        DataType::SocialPost,
        "BTC-USD",
    );
    let parsed = parse_subject(&subject).unwrap();
    assert_eq!(parsed.data_type_key, "social.post");
    assert_eq!(parsed.venue_slug, "kraken");
    assert_eq!(parsed.instrument_id, "BTC-USD");
}

#[test]
fn all_data_types_produce_parseable_subjects() {
    for &dt in DataType::all() {
        let subject = data_subject(
            SupportedVenue::Kraken,
            AssetClass::CryptoSpotCex,
            dt,
            "BTC-USD",
        );
        let parsed = parse_subject(&subject)
            .unwrap_or_else(|| panic!("failed to parse subject for {:?}: {}", dt, subject));
        assert_eq!(parsed.data_type_key, dt.as_key());
    }
}

#[test]
fn dex_quote_subject_for_zerox() {
    let s = data_subject(
        SupportedVenue::ZeroX,
        AssetClass::CryptoSpotDex,
        DataType::DexQuote,
        "WETH-USDC",
    );
    assert_eq!(s, "md.dex.quote.zerox.crypto_spot_dex.WETH-USDC");
}
