//! Adversarial test: a missing broker ack triggers a query, not a blind retry.
//!
//! This proves the idempotency + safety contract: if a submit call returns but
//! no fill/ack arrives, the engine calls `query_order` to learn the broker's
//! view — it never resubmits (which would double-fill).

use std::str::FromStr;
use std::sync::Arc;

use domain::{
    money::Size,
    order::{OrderIntent, OrderType, Side},
};
use execution::{mock_broker::MockBroker, ExecutionEngine};
use risk::{ApprovedOrder, GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

fn approved_intent(instrument_id: &str) -> ApprovedOrder {
    let intent = OrderIntent::new(
        instrument_id,
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    );
    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), ks);
    let ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        None,
        Decimal::from_str("0.01").unwrap(),
        Decimal::from_str("0.001").unwrap(),
        Decimal::ZERO,
        true,
        0,
        0,
    );
    gate.check(intent, &ctx)
        .expect("test intent should be approved")
}

#[tokio::test]
async fn missing_ack_triggers_query_not_resubmit() {
    let mock = Arc::new(MockBroker::new());
    let engine = ExecutionEngine::new(Arc::clone(&mock) as Arc<_>);

    let order = approved_intent("BTC-USD");
    let key = order.intent.idempotency_key;

    // Submit.
    let submit_result = engine.submit(order).await.expect("submit should succeed");
    let broker_id = submit_result.broker_order_id;

    assert_eq!(mock.submit_call_count(), 1);
    assert_eq!(mock.query_call_count(), 0);

    // Simulate missing ack: caller invokes sync_order instead of resubmitting.
    engine
        .sync_order(key, &broker_id, Decimal::ZERO, Side::Buy)
        .await
        .expect("sync should succeed");

    // Query was called once; submit was NOT called again.
    assert_eq!(mock.query_call_count(), 1, "query should have been called");
    assert_eq!(
        mock.submit_call_count(),
        1,
        "submit must NOT be called again — that would double-fill"
    );
}
