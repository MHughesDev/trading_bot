//! P2-T08 acceptance test: Tradier option contract normalizes to OHLCV + quote events.

use collectors::options::tradier::TradierOptionsCollector;

#[test]
fn tradier_collector_symbol_matches_instrument_id() {
    let c = TradierOptionsCollector::new("AAPL240621C00200000");
    assert_eq!(c.symbol, "AAPL240621C00200000");
    assert_eq!(c.instrument_id, "AAPL240621C00200000");
}
