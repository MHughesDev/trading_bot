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
use execution::journal::{self, memory::MemoryOrderJournal};
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
        // A measured mark: the gate refuses an order it cannot price.
        Some(domain::money::Price::from_str("50000").unwrap()),
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
    // An order that cannot be recorded is not submitted, so the engine needs a
    // journal.  This suite has no database, so it uses the in-memory one.
    journal::install(Arc::new(MemoryOrderJournal::new()));

    let mock = Arc::new(MockBroker::new());
    let engine = ExecutionEngine::new(Arc::clone(&mock) as Arc<_>);

    let order = approved_intent("BTC-USD");
    let key = order.intent.idempotency_key;

    // Submit.
    let submit_result = engine.submit(order).await.expect("submit should succeed");
    let broker_id = submit_result.broker_order_id;

    assert_eq!(mock.submit_call_count(), 1);
    // `submit` reads the order back so an immediate fill is captured and
    // recorded; that read is a query, and it is the only extra venue call.
    let queries_after_submit = mock.query_call_count();

    // Simulate missing ack: caller invokes sync_order instead of resubmitting.
    engine
        .sync_order(key, &broker_id, Decimal::ZERO, Side::Buy)
        .await
        .expect("sync should succeed");

    // The response to a missing ack is one more query, never another submit.
    assert_eq!(
        mock.query_call_count(),
        queries_after_submit + 1,
        "a missing ack must be answered by querying the broker"
    );
    assert_eq!(
        mock.submit_call_count(),
        1,
        "submit must NOT be called again — that would double-fill"
    );
}
