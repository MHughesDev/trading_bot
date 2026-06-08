//! Adversarial test: loosening overrides are rejected; tightening ones are applied.

use std::str::FromStr;

use domain::strategy_def::risk_overrides::RiskOverrides;
use domain::ValidationError;
use risk::{limits::GlobalRiskLimits, overrides::apply_tighten_only};
use rust_decimal::Decimal;

fn global() -> GlobalRiskLimits {
    GlobalRiskLimits {
        max_position: Decimal::from(10),
        max_orders_per_second: 5,
        max_orders_per_minute: 60,
        price_band_bps: 500,
        max_daily_loss_usd: Decimal::from(1000),
    }
}

#[test]
fn loosening_max_position_is_rejected() {
    let overrides = RiskOverrides {
        max_position: Some(Decimal::from(100)), // 10x the global limit
        ..Default::default()
    };
    let err = apply_tighten_only(&global(), &overrides).unwrap_err();
    assert!(
        matches!(err, ValidationError::RiskOverrideTooPermissive { field } if field == "max_position")
    );
}

#[test]
fn tightening_max_position_is_applied() {
    let overrides = RiskOverrides {
        max_position: Some(Decimal::from(5)), // half the global limit
        ..Default::default()
    };
    let effective = apply_tighten_only(&global(), &overrides).unwrap();
    assert_eq!(effective.max_position, Decimal::from(5));
}

#[test]
fn loosening_rate_per_second_is_rejected() {
    let overrides = RiskOverrides {
        max_order_rate_per_second: Some(100), // above global 5
        ..Default::default()
    };
    let err = apply_tighten_only(&global(), &overrides).unwrap_err();
    assert!(matches!(
        err,
        ValidationError::RiskOverrideTooPermissive { field } if field == "max_order_rate_per_second"
    ));
}

#[test]
fn tightening_rate_per_minute_is_applied() {
    let overrides = RiskOverrides {
        max_order_rate_per_minute: Some(10), // below global 60
        ..Default::default()
    };
    let effective = apply_tighten_only(&global(), &overrides).unwrap();
    assert_eq!(effective.max_orders_per_minute, 10);
}

#[test]
fn empty_overrides_leaves_global_unchanged() {
    let g = global();
    let effective = apply_tighten_only(&g, &RiskOverrides::default()).unwrap();
    assert_eq!(effective.max_position, g.max_position);
    assert_eq!(effective.max_orders_per_second, g.max_orders_per_second);
    assert_eq!(effective.max_orders_per_minute, g.max_orders_per_minute);
}

#[test]
fn low_trust_intent_refused_by_gate() {
    use domain::RiskRejection;
    use domain::{
        money::Size,
        order::{OrderIntent, OrderType, Side},
        TrustTier,
    };
    use risk::{GateContext, KillSwitch, RiskGate};
    use std::sync::Arc;

    let gate = RiskGate::new(
        GlobalRiskLimits::default(),
        Arc::new(KillSwitch::new(false)),
    );
    let intent = OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    );
    let mut ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        None,
        Decimal::from_str("0.01").unwrap(),
        Decimal::from_str("0.001").unwrap(),
        Decimal::ZERO,
        true,
        0,
        0,
    );
    ctx.event_trust_tier = TrustTier::SocialDerived;
    ctx.strategy_min_trust_tier = TrustTier::CentralizedExchange;

    let err = gate.check(intent, &ctx).unwrap_err();
    assert!(matches!(err, RiskRejection::TrustTierInsufficient { .. }));
}
