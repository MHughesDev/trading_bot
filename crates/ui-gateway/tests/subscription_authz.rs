//! Adversarial tests: per-subscription authorization for private vs public lanes.

use std::sync::Arc;

use demand_manager::{DemandRegistry, NoopPipelineFactory};
use ui_gateway::{SubscriptionError, SubscriptionRegistry};

fn registry() -> SubscriptionRegistry {
    let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
    SubscriptionRegistry::new(demand)
}

#[test]
fn user_can_subscribe_own_private_lane() {
    let reg = registry();
    let result = reg.subscribe(
        "panel_1",
        "alice",
        "orders.events",
        "BTC-USD",
        "alice", // requesting as alice
        None,
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn user_cannot_subscribe_another_users_private_lane() {
    let reg = registry();
    let result = reg.subscribe(
        "panel_1",
        "alice", // subscription owned by alice
        "orders.events",
        "BTC-USD",
        "bob", // but requested by bob — must fail
        None,
        None,
    );
    assert!(
        matches!(result, Err(SubscriptionError::Unauthorized { .. })),
        "expected Unauthorized, got {result:?}",
    );
}

#[test]
fn public_lane_is_shareable() {
    let reg = registry();
    // Bob can subscribe to Alice's public data (it's not scoped per user anyway).
    let result = reg.subscribe(
        "panel_2",
        "alice",
        "market.bars.1m",
        "AAPL",
        "bob",
        None,
        None,
    );
    assert!(result.is_ok());
}

#[test]
fn unknown_lane_is_rejected() {
    let reg = registry();
    let result = reg.subscribe(
        "panel_3",
        "alice",
        "not.a.real.lane",
        "BTC-USD",
        "alice",
        None,
        None,
    );
    assert!(matches!(result, Err(SubscriptionError::UnknownLane(_))));
}

#[test]
fn ui_orderbook_snapshot_is_accepted() {
    let reg = registry();
    let result = reg.subscribe(
        "ob_panel",
        "alice",
        "ui.orderbook.snapshot",
        "BTC-USD",
        "alice",
        Some(20),
        Some(20),
    );
    assert!(result.is_ok());
}

#[test]
fn subscribe_increments_demand_remove_decrements() {
    let reg = registry();
    assert_eq!(reg.demand_count("market.bars.1m", "ETH-USD"), 0);

    let sub = reg
        .subscribe(
            "panel_a",
            "alice",
            "market.bars.1m",
            "ETH-USD",
            "alice",
            None,
            None,
        )
        .unwrap();

    assert_eq!(reg.demand_count("market.bars.1m", "ETH-USD"), 1);

    reg.remove(sub.id);
    assert_eq!(reg.demand_count("market.bars.1m", "ETH-USD"), 0);
}

#[test]
fn remove_all_for_user_cleans_up_demand() {
    let reg = registry();

    reg.subscribe(
        "panel_1",
        "alice",
        "market.bars.1m",
        "BTC-USD",
        "alice",
        None,
        None,
    )
    .unwrap();
    reg.subscribe(
        "panel_2",
        "alice",
        "market.trades",
        "BTC-USD",
        "alice",
        None,
        None,
    )
    .unwrap();

    assert_eq!(reg.demand_count("market.bars.1m", "BTC-USD"), 1);
    assert_eq!(reg.demand_count("market.trades", "BTC-USD"), 1);

    reg.remove_all_for_user("alice");

    assert_eq!(reg.demand_count("market.bars.1m", "BTC-USD"), 0);
    assert_eq!(reg.demand_count("market.trades", "BTC-USD"), 0);
}
