//! Adversarial unit tests: two consumers sharing a lane keep one pipeline;
//! when both unsubscribe the pipeline stops; partial unsubscribe keeps it alive.

use std::sync::Arc;

use demand_manager::{DemandRegistry, NoopPipelineFactory};
use domain::lanes::Lane;

fn registry() -> DemandRegistry {
    DemandRegistry::new(Arc::new(NoopPipelineFactory))
}

#[test]
fn first_add_increments_count_to_one() {
    let reg = registry();
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 0);
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 1);
}

#[test]
fn two_consumers_same_lane_both_tracked() {
    let reg = registry();
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 2);
}

#[test]
fn partial_unsubscribe_keeps_pipeline_alive() {
    let reg = registry();
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    reg.remove(&Lane::MarketBars1m, "BTC-USD");
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 1);
}

#[test]
fn last_unsubscribe_stops_pipeline() {
    let reg = registry();
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    reg.remove(&Lane::MarketBars1m, "BTC-USD");
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 0);
}

#[test]
fn different_instruments_are_independent() {
    let reg = registry();
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    reg.add(&Lane::MarketBars1m, "ETH-USD");
    reg.remove(&Lane::MarketBars1m, "BTC-USD");
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 0);
    assert_eq!(reg.count(&Lane::MarketBars1m, "ETH-USD"), 1);
}

#[test]
fn different_lanes_are_independent() {
    let reg = registry();
    reg.add(&Lane::MarketBars1m, "BTC-USD");
    reg.add(&Lane::MarketTrades, "BTC-USD");
    reg.remove(&Lane::MarketBars1m, "BTC-USD");
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 0);
    assert_eq!(reg.count(&Lane::MarketTrades, "BTC-USD"), 1);
}

#[test]
fn remove_with_no_demand_is_noop() {
    let reg = registry();
    reg.remove(&Lane::MarketBars1m, "BTC-USD"); // no panic
    assert_eq!(reg.count(&Lane::MarketBars1m, "BTC-USD"), 0);
}
