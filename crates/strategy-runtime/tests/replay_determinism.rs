//! Proves that the same event sequence yields structurally identical order intents
//! across two independent replay runs — the strategy is deterministic.

use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use domain::money::{Price, Size};
use domain::order::Side;
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::strategy_def::{
    actions::{Action, ActionKind, OrderSpec, SizeMode},
    inputs::InputDeclaration,
    nodes::{Node, NodeKind},
    risk_overrides::RiskOverrides,
    StrategyDefinition,
};
use features::FeatureValue;
use strategy_runtime::{StrategyClock, StrategyInstance, WallClock, WorldEvent};

fn ema_cross_def() -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "ema_cross_v1".into(),
        definition_version: "1.0".into(),
        asset_class: "crypto_spot_cex".into(),
        min_trust_tier: domain::TrustTier::CentralizedExchange,
        inputs: vec![
            InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            },
            InputDeclaration {
                lane: "features.technical".into(),
                instrument: "$bound_at_init".into(),
                features: vec!["ema_7".into(), "ema_21".into()],
            },
        ],
        nodes: vec![
            Node {
                id: "n1".into(),
                kind: NodeKind::Condition {
                    expr: "feature('ema_7') > feature('ema_21')".into(),
                },
            },
            Node {
                id: "n2".into(),
                kind: NodeKind::Signal {
                    when: "n1".into(),
                    emit: "long".into(),
                },
            },
        ],
        actions: vec![Action {
            on_signal: "long".into(),
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

fn make_events() -> Vec<WorldEvent> {
    let t = Utc::now();
    let bar = BarPayload::new(
        Timeframe::Minutes1,
        Price::from_str("100").unwrap(),
        Price::from_str("110").unwrap(),
        Price::from_str("95").unwrap(),
        Price::from_str("105").unwrap(),
        Size::from_str("500").unwrap(),
        200,
    );

    vec![
        // Inject feature values that satisfy ema_7 > ema_21
        WorldEvent::Feature {
            instrument_id: "BTC-USDT".into(),
            feature_value: FeatureValue::new("ema_7", 11.0, 1, t),
        },
        WorldEvent::Feature {
            instrument_id: "BTC-USDT".into(),
            feature_value: FeatureValue::new("ema_21", 10.0, 1, t),
        },
        // A bar event triggers evaluation
        WorldEvent::Bar {
            instrument_id: "BTC-USDT".into(),
            timeframe: Timeframe::Minutes1,
            bar,
            available_time: t,
        },
    ]
}

fn run_once(events: Vec<WorldEvent>) -> Vec<domain::order::OrderIntent> {
    let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
    let mut instance = StrategyInstance::new("user1", "BTC-USDT", ema_cross_def(), clock.now());
    let mut all_intents = Vec::new();
    for event in &events {
        all_intents.extend(instance.process_event(&event));
    }
    all_intents
}

#[test]
fn identical_events_produce_identical_intents() {
    let events = make_events();
    let a = run_once(events.clone());
    let b = run_once(events);

    assert_eq!(a.len(), b.len(), "intent counts must match across runs");
    assert!(
        !a.is_empty(),
        "the EMA-cross condition must fire at least once"
    );

    for (ia, ib) in a.iter().zip(b.iter()) {
        assert_eq!(ia.instrument_id, ib.instrument_id);
        assert_eq!(ia.side, ib.side);
        assert_eq!(ia.size, ib.size);
        assert_eq!(ia.strategy_id, ib.strategy_id);
        assert_eq!(ia.order_type, ib.order_type);
        // idempotency_key is intentionally not compared — it is a random UUID
    }
}

#[test]
fn condition_false_emits_no_intents() {
    let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
    let mut instance = StrategyInstance::new("user1", "BTC-USDT", ema_cross_def(), clock.now());

    // ema_7 < ema_21 — condition is false
    let t = Utc::now();
    instance.process_event(&WorldEvent::Feature {
        instrument_id: "BTC-USDT".into(),
        feature_value: FeatureValue::new("ema_7", 9.0, 1, t),
    });
    instance.process_event(&WorldEvent::Feature {
        instrument_id: "BTC-USDT".into(),
        feature_value: FeatureValue::new("ema_21", 10.0, 1, t),
    });

    let bar = BarPayload::new(
        Timeframe::Minutes1,
        Price::from_str("100").unwrap(),
        Price::from_str("110").unwrap(),
        Price::from_str("95").unwrap(),
        Price::from_str("105").unwrap(),
        Size::from_str("500").unwrap(),
        200,
    );
    let intents = instance.process_event(&WorldEvent::Bar {
        instrument_id: "BTC-USDT".into(),
        timeframe: Timeframe::Minutes1,
        bar,
        available_time: t,
    });

    assert!(intents.is_empty(), "no intents when condition is false");
}
