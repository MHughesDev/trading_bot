//! Proves that `world.now()` returns the event's `available_time`, not the OS clock.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{Duration, Utc};
use domain::money::{Price, Size};
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use domain::order::Side;
use strategy_runtime::{StrategyClock, WallClock, WorldEvent, StrategyInstance};

fn minimal_def() -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "test".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: domain::TrustTier::CentralizedExchange,
        inputs: vec![InputDeclaration {
            lane: "market.bars.1m".into(),
            instrument: "$bound_at_init".into(),
            features: vec![],
        }],
        nodes: vec![Node {
            id: "n1".into(),
            kind: NodeKind::Condition {
                expr: "1.0 > 0.0".into(),
            },
        }],
        actions: vec![Action {
            on_signal: "sig".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: SizeMode::Fixed,
                    size: "0.01".into(),
                },
            },
        }],
        risk_overrides: RiskOverrides::default(),
    }
}

fn make_bar() -> BarPayload {
    BarPayload::new(
        Timeframe::Minutes1,
        Price::from_str("100").unwrap(),
        Price::from_str("110").unwrap(),
        Price::from_str("95").unwrap(),
        Price::from_str("105").unwrap(),
        Size::from_str("500").unwrap(),
        200,
    )
}

#[test]
fn current_time_tracks_event_available_time_not_wall_clock() {
    // Use a time approximately one hour in the past.
    let past_time = Utc::now() - Duration::hours(1);

    let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
    let mut instance = StrategyInstance::new("user1", "BTC-USDT", minimal_def(), clock.now());

    let event = WorldEvent::Bar {
        instrument_id: "BTC-USDT".into(),
        timeframe: Timeframe::Minutes1,
        bar: make_bar(),
        available_time: past_time,
    };

    instance.process_event(event);

    // After processing, current_time must equal the event's available_time.
    assert_eq!(
        instance.current_time(),
        past_time,
        "current_time must equal the event's available_time"
    );

    // Verify it is well behind the wall clock (≥ 59 minutes behind).
    let wall_now = Utc::now();
    let lag_minutes = (wall_now - instance.current_time())
        .num_minutes()
        .unsigned_abs();
    assert!(
        lag_minutes >= 59,
        "world.now() must be driven by event available_time, not wall clock; \
         lag was only {lag_minutes}m"
    );
}
