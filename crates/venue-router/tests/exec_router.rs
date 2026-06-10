//! P4-T02 acceptance tests — execution router.

use std::sync::Arc;

use domain::{
    instrument::AssetClass,
    money::{Price, Size},
    order::{OrderIntent, OrderType, Side},
};
use execution::mock_broker::MockBroker;
use risk::ApprovedOrder;
use rust_decimal_macros::dec;
use venue_router::{ExecRouter, ExecutionMode, RouteOutcome};

fn make_approved_order(instrument_id: &str, side: Side) -> ApprovedOrder {
    let intent = OrderIntent::new(
        instrument_id,
        side,
        OrderType::Market,
        Size::from_decimal(dec!(1)),
        None,
        None,
    );
    ApprovedOrder::new_for_test(intent)
}

/// Paper crypto order → `CLOBFillSimulator` — no network.
#[tokio::test]
async fn paper_crypto_routes_to_clob_simulator() {
    let router = ExecRouter::new();
    let approved = make_approved_order("BTC-USD", Side::Buy);
    let mark = Price::from_decimal(dec!(50000));

    let outcome = router
        .route(
            ExecutionMode::Paper,
            AssetClass::CryptoSpotCex,
            "coinbase",
            &approved.intent,
            mark,
            &approved,
        )
        .await
        .expect("paper crypto routing must succeed");

    match outcome {
        RouteOutcome::PaperFill(fill) => {
            assert!(fill.filled_qty > dec!(0), "must have non-zero fill");
            // No broker was touched.
        }
        RouteOutcome::LiveSubmitted { .. } => panic!("paper order should not go to live adapter"),
    }
}

/// LiveRouted equity order → Alpaca adapter.
#[tokio::test]
async fn live_routed_equity_routes_to_alpaca_adapter() {
    let mut router = ExecRouter::new();
    let mock = Arc::new(MockBroker::new());
    router.register(
        "alpaca",
        Arc::clone(&mock) as Arc<dyn execution::broker::Broker>,
    );

    let approved = make_approved_order("AAPL", Side::Buy);
    let mark = Price::from_decimal(dec!(175));

    let outcome = router
        .route(
            ExecutionMode::LiveRouted,
            AssetClass::Equity,
            "alpaca",
            &approved.intent,
            mark,
            &approved,
        )
        .await
        .expect("live routing must succeed");

    match outcome {
        RouteOutcome::LiveSubmitted { broker_order_id } => {
            assert!(!broker_order_id.is_empty());
            assert_eq!(mock.submit_call_count(), 1, "broker submit must be called");
        }
        RouteOutcome::PaperFill(_) => panic!("live order should not return paper fill"),
    }
}

/// LiveRouted with no adapter registered returns `NoAdapter` error (NOT a risk error).
#[tokio::test]
async fn live_routed_no_adapter_returns_routing_error() {
    let router = ExecRouter::new(); // no adapters registered
    let approved = make_approved_order("EUR-USD", Side::Buy);
    let mark = Price::from_decimal(dec!(1.08));

    let result = router
        .route(
            ExecutionMode::LiveRouted,
            AssetClass::Fx,
            "oanda",
            &approved.intent,
            mark,
            &approved,
        )
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let msg = err.to_string();
    // Error message must not say "risk".
    assert!(
        !msg.to_lowercase().contains("risk"),
        "routing error must not say 'risk': {msg}"
    );
}
