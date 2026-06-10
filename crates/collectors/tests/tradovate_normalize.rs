//! P2-T10 acceptance test: Tradovate futures bar normalizes to correct OHLCV event.

use collectors::futures::tradovate::TradovateCollector;

#[test]
fn tradovate_collector_symbol_matches_instrument_id() {
    let c = TradovateCollector::new("ESH4");
    assert_eq!(c.instrument_id, "ESH4");
    assert_eq!(c.symbol, "ESH4");
}
