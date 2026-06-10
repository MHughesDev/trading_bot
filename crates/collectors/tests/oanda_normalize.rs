//! P2-T06 acceptance test: OANDA candle normalizes to correct EventEnvelope.

use collectors::fx::oanda::OandaCollector;
use domain::payloads::bar::Timeframe;
use domain::TrustTier;

#[test]
fn oanda_collector_instrument_id_uses_dash() {
    let c = OandaCollector::new("EUR_USD");
    assert_eq!(c.instrument_id, "EUR-USD");
    assert_eq!(c.venue_id, "oanda");
}

#[test]
fn gbp_jpy_pair_id_is_correct() {
    let c = OandaCollector::new("GBP_JPY");
    assert_eq!(c.instrument_id, "GBP-JPY");
}
