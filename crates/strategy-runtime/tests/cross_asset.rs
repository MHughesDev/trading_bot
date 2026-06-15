//! Phase 6 cross-asset abstraction proof.
//!
//! The invariant: the same strategy definition runs identically across a crypto
//! instrument (BTC-USD, 24/7, non-haltable) and an equity instrument (AAPL,
//! session-bound, haltable) through **identical core code** — no asset-class
//! branch in the runtime, risk gate, or strategy evaluator.  Only metadata
//! (trading_hours, halt_policy, trust_tier) differs; the StrategyInstance,
//! WorldState, bytecode compiler, and signal emission logic are unchanged.

use chrono::Utc;
use domain::{
    instrument::{HaltPolicy, TradingSchedule, TradingSession},
    money::{Price, Size},
    payloads::bar::{BarPayload, Timeframe},
    strategy_def::{
        actions::{Action, ActionKind, OrderSpec, SizeMode},
        inputs::InputDeclaration,
        nodes::{Node, NodeKind},
        StrategyDefinition,
    },
    TrustTier,
};
use features::FeatureValue;
use reconciliation::freshness::{check_freshness, is_within_trading_hours, FreshnessOutcome};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use std::sync::Arc;
use rust_decimal::Decimal;
use strategy_runtime::{StrategyInstance, WorldEvent};

// ── shared strategy definition ─────────────────────────────────────────────

/// A simple EMA-cross definition with `asset_class = "any"` (or crypto) so it
/// is accepted by both crypto and equity instances.  The core runtime does not
/// gate on asset_class — that is validated at definition upload, not at runtime.
fn ema_cross_def(asset_class: &str) -> StrategyDefinition {
    StrategyDefinition {
        strategy_id: "ema_cross_cross_asset".into(),
        definition_version: "1.0".into(),
        asset_class: asset_class.into(),
        min_trust_tier: TrustTier::CentralizedExchange,
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
                    side: domain::order::Side::Buy,
                    size_mode: SizeMode::Fixed,
                    size: "0.01".to_string(),
                },
            },
        }],
        risk_overrides: Default::default(),
    }
}

/// Feature event that fires the EMA-cross condition (ema_7 > ema_21).
fn ema_feature_event(name: &str, value: f64, instrument: &str) -> WorldEvent {
    WorldEvent::Feature {
        instrument_id: instrument.to_string(),
        feature_value: FeatureValue {
            name: name.to_string(),
            value,
            feature_version: 0,
            available_time: Utc::now(),
        },
    }
}

fn bar_event(instrument: &str) -> WorldEvent {
    WorldEvent::Bar {
        instrument_id: instrument.to_string(),
        timeframe: Timeframe::Minutes1,
        bar: BarPayload {
            timeframe: Timeframe::Minutes1,
            open: Price::from_decimal(Decimal::new(100, 0)),
            high: Price::from_decimal(Decimal::new(101, 0)),
            low: Price::from_decimal(Decimal::new(99, 0)),
            close: Price::from_decimal(Decimal::new(100, 0)),
            volume: Size::from_decimal(Decimal::new(1000, 0)),
            trade_count: 10,
            revision: 0,
        },
        available_time: Utc::now(),
    }
}

// ── P6-T04a: identical signal emission across asset classes ────────────────

/// Same definition, same event sequence → same signals on both crypto (BTC-USD)
/// and equity (AAPL).  Proves no asset-class branch in StrategyInstance.
#[test]
fn same_strategy_fires_same_signals_on_crypto_and_equity() {
    let now = Utc::now();

    // Crypto instance (BTC-USD)
    let mut crypto_instance = StrategyInstance::new(
        "user1",
        "BTC-USD",
        ema_cross_def("crypto_spot_cex"),
        now,
    );
    // Equity instance (AAPL)
    let mut equity_instance = StrategyInstance::new(
        "user1",
        "AAPL",
        ema_cross_def("equity"),
        now,
    );

    // Feed identical feature values to both (ema_7 crosses above ema_21).
    let events: Vec<WorldEvent> = vec![
        ema_feature_event("ema_7", 105.0, "BTC-USD"),
        ema_feature_event("ema_21", 100.0, "BTC-USD"),
        bar_event("BTC-USD"),
    ];
    let equity_events: Vec<WorldEvent> = vec![
        ema_feature_event("ema_7", 105.0, "AAPL"),
        ema_feature_event("ema_21", 100.0, "AAPL"),
        bar_event("AAPL"),
    ];

    let mut crypto_intents = vec![];
    for event in &events {
        crypto_intents.extend(crypto_instance.process_event(event));
    }

    let mut equity_intents = vec![];
    for event in &equity_events {
        equity_intents.extend(equity_instance.process_event(event));
    }

    // Both emit the same signal (long) → same intent shape (Buy, fixed 0.01).
    assert_eq!(
        crypto_intents.len(),
        equity_intents.len(),
        "crypto and equity must emit identical intent counts for the same feature values"
    );
    assert!(!crypto_intents.is_empty(), "EMA cross should emit a long intent");

    let c = &crypto_intents[0];
    let e = &equity_intents[0];
    assert_eq!(c.side, e.side, "same side");
    assert_eq!(c.order_type, e.order_type, "same order type");
    assert_eq!(c.size, e.size, "same size");
}

// ── P6-T04b: freshness watchdog respects equity session ─────────────────────

/// Outside the NYSE session window, `check_freshness` must return `StaleOutsideHours`
/// (not `StaleAlarm`) even when data is hours old — the instrument is simply closed.
/// This proves the watchdog does not false-alarm on an equity instrument outside
/// its trading hours.
#[test]
fn freshness_no_false_alarm_outside_equity_session() {
    let nyse = TradingSchedule {
        timezone: "America/New_York".to_owned(),
        sessions: vec![TradingSession {
            open: "09:30".to_owned(),
            close: "16:00".to_owned(),
        }],
        has_pre_market: false,
        has_post_market: false,
    };

    // Simulate: last data 3 hours ago, but we are outside the session window.
    // Use a fixed UTC time that corresponds to 20:00 New York (after session close).
    // 20:00 ET = 01:00 UTC next day (EST) or 00:00 UTC (EDT).
    // Using a simple midnight UTC — 00:00 UTC = 20:00 EST.
    let after_close = chrono::DateTime::parse_from_rfc3339("2026-06-15T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let three_hours_ago = after_close - chrono::Duration::hours(3);

    let kill_switch = Arc::new(KillSwitch::new(false));
    let outcome = check_freshness(
        "AAPL",
        "market.trades",
        three_hours_ago,
        after_close,
        &nyse,
        60,
        &kill_switch,
    );

    // Outside the NYSE session: watchdog must not fire (data may be stale
    // simply because the market is closed, not because of a feed outage).
    assert_eq!(
        outcome,
        FreshnessOutcome::StaleOutsideHours,
        "freshness must return StaleOutsideHours outside NYSE hours"
    );
    assert!(
        !kill_switch.is_active(),
        "kill switch must not trip on stale data outside trading hours"
    );
}

/// Inside the NYSE session, stale data (>watermark) must trigger the watchdog.
#[test]
fn freshness_alarms_on_stale_data_during_equity_session() {
    let nyse = TradingSchedule {
        timezone: "America/New_York".to_owned(),
        sessions: vec![TradingSession {
            open: "09:30".to_owned(),
            close: "16:00".to_owned(),
        }],
        has_pre_market: false,
        has_post_market: false,
    };

    // 14:00 UTC = 10:00 ET (within NYSE session)
    let mid_session = chrono::DateTime::parse_from_rfc3339("2026-06-15T14:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let stale = mid_session - chrono::Duration::minutes(5);

    let kill_switch = Arc::new(KillSwitch::new(false));
    let outcome = check_freshness(
        "AAPL",
        "market.trades",
        stale,
        mid_session,
        &nyse,
        60, // 60 second watermark
        &kill_switch,
    );

    assert!(
        matches!(outcome, FreshnessOutcome::StaleAlarm { .. }),
        "stale data during session must trigger StaleAlarm, got {outcome:?}"
    );
}

// ── P6-T04c: is_within_trading_hours is session-aware ──────────────────────

#[test]
fn is_within_trading_hours_returns_true_for_crypto_24_7() {
    let crypto = TradingSchedule::always_open();
    let any_time = Utc::now();
    assert!(
        is_within_trading_hours(any_time, &crypto),
        "crypto 24/7 schedule must always be in session"
    );
}

#[test]
fn is_within_trading_hours_returns_false_outside_nyse() {
    let nyse = TradingSchedule {
        timezone: "America/New_York".to_owned(),
        sessions: vec![TradingSession {
            open: "09:30".to_owned(),
            close: "16:00".to_owned(),
        }],
        has_pre_market: false,
        has_post_market: false,
    };

    // 00:00 UTC = 20:00 EST — well after close
    let after_close = chrono::DateTime::parse_from_rfc3339("2026-06-15T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    assert!(
        !is_within_trading_hours(after_close, &nyse),
        "00:00 UTC (20:00 ET) must be outside NYSE hours"
    );

    // 14:00 UTC = 10:00 ET — inside session
    let mid_session = chrono::DateTime::parse_from_rfc3339("2026-06-15T14:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    assert!(
        is_within_trading_hours(mid_session, &nyse),
        "14:00 UTC (10:00 ET) must be inside NYSE hours"
    );
}

// ── P6-T04d: risk gate rejects halted/out-of-session equity orders ─────────

/// Proves the risk gate (unchanged core code) correctly rejects equity orders
/// outside session and during a halt — no asset-class branch needed.
#[test]
fn risk_gate_rejects_equity_outside_session() {
    use domain::order::{OrderIntent, OrderType, Side};

    let kill_switch = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&kill_switch));

    let intent = OrderIntent::new(
        "AAPL".to_string(),
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(Decimal::new(1, 0)),
        None,
        None,
    );

    let mut ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_decimal(Decimal::new(150, 0))),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        true,
        0,
        0,
    );
    ctx.is_in_session = false;
    ctx.halt_policy = HaltPolicy::Haltable;

    let result = gate.check(intent, &ctx);
    assert!(result.is_err(), "order outside session must be rejected");
    match result {
        Err(domain::RiskRejection::OutsideTradingHours { .. }) => {}
        other => panic!("expected OutsideTradingHours, got {other:?}"),
    }
}

#[test]
fn risk_gate_rejects_halted_equity() {
    use domain::order::{OrderIntent, OrderType, Side};

    let kill_switch = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&kill_switch));

    let intent = OrderIntent::new(
        "AAPL".to_string(),
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(Decimal::new(1, 0)),
        None,
        None,
    );

    let mut ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_decimal(Decimal::new(150, 0))),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        true,
        0,
        0,
    );
    ctx.is_in_session = true;
    ctx.halt_policy = HaltPolicy::Haltable;
    ctx.is_halted = true;

    let result = gate.check(intent, &ctx);
    assert!(result.is_err(), "halted equity order must be rejected");
    match result {
        Err(domain::RiskRejection::InstrumentHalted { .. }) => {}
        other => panic!("expected InstrumentHalted, got {other:?}"),
    }
}

#[test]
fn risk_gate_approves_equity_in_session_not_halted() {
    use domain::order::{OrderIntent, OrderType, Side};

    let kill_switch = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&kill_switch));

    let intent = OrderIntent::new(
        "AAPL".to_string(),
        Side::Buy,
        OrderType::Market,
        Size::from_decimal(Decimal::new(1, 0)),
        None,
        None,
    );

    let mut ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        Some(Price::from_decimal(Decimal::new(150, 0))),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        true,
        0,
        0,
    );
    ctx.is_in_session = true;
    ctx.halt_policy = HaltPolicy::Haltable;
    ctx.is_halted = false;

    let result = gate.check(intent, &ctx);
    assert!(result.is_ok(), "in-session, non-halted equity order must be approved");
}
