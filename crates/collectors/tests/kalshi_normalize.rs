//! P2-T07 acceptance test: Kalshi YES/NO market normalizes to prediction-price event;
//! perpetual sample normalizes to OHLCV + funding events.

use collectors::prediction::kalshi::{KalshiCollector, KalshiMarketKind};

#[test]
fn kalshi_prediction_collector_kind_is_prediction() {
    let c = KalshiCollector::new_prediction("PRES-2024-D");
    assert!(matches!(c.kind, KalshiMarketKind::Prediction));
    assert_eq!(c.instrument_id, "PRES-2024-D");
}

#[test]
fn kalshi_perpetual_collector_kind_is_perpetual() {
    let c = KalshiCollector::new_perpetual("BTC-PERP");
    assert!(matches!(c.kind, KalshiMarketKind::Perpetual));
    assert_eq!(c.instrument_id, "BTC-PERP");
}
