//! In-house paper broker — a per-asset-class [`Broker`] view of the
//! [`PaperTradingEngine`].
//!
//! No external API calls are ever made.  Fills are simulated at the latest
//! internal mark price and applied to the engine's internal account for this
//! asset class; orders, positions, and balances are all queryable from the
//! same internal state.  This is the default execution path for every account
//! that has not loaded live trading credentials from the database.

use std::sync::Arc;

use async_trait::async_trait;
use domain::instrument::AssetClass;
use risk::ApprovedOrder;
use tracing::info;

use super::simulator_for;
use crate::broker::{Broker, BrokerError, BrokerOrderStatus, BrokerPosition};

use super::engine::PaperTradingEngine;

/// `Broker` implementation backed entirely by the internal paper engine.
#[derive(Clone)]
pub struct PaperBroker {
    engine: Arc<PaperTradingEngine>,
    asset_class: AssetClass,
}

impl PaperBroker {
    /// Broker view over an existing engine (see [`PaperTradingEngine::broker`]).
    pub fn for_engine(engine: Arc<PaperTradingEngine>, asset_class: AssetClass) -> Self {
        Self {
            engine,
    /// Create a paper broker and return the shared mark-price handle.
    ///
    /// The caller should keep the returned `Arc<RwLock<Price>>` and write
    /// fresh prices into it as ticks arrive. The broker reads it on each `submit`.
    pub fn new(asset_class: AssetClass) -> (Self, Arc<RwLock<Price>>) {
        let mark = Arc::new(RwLock::new(Price::from_decimal(Decimal::ONE)));
        let broker = Self {
            mark: Arc::clone(&mark),
            asset_class,
        }
    }

    /// Convenience constructor for a self-contained broker: builds a fresh
    /// engine and returns it alongside the broker so the caller can feed
    /// marks via [`PaperTradingEngine::on_mark`] and read account state.
    pub fn new(asset_class: AssetClass) -> (Self, Arc<PaperTradingEngine>) {
        let engine = Arc::new(PaperTradingEngine::new());
        (engine.broker(asset_class), engine)
    }

    pub fn asset_class(&self) -> AssetClass {
        self.asset_class
    }
}

#[async_trait]
impl Broker for PaperBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let order_id = self
            .engine
            .submit(self.asset_class, &order.intent)
            .map_err(|e| BrokerError::Rejected(e.to_string()))?;

        if let Some(status) = self.engine.order_status(&order_id) {
            info!(
                %order_id,
                instrument_id = %order.intent.instrument_id,
                side          = ?order.intent.side,
                state         = ?status.state,
                filled_qty    = %status.filled_qty,
                "paper order processed (in-house)"
            );
        }
        Ok(order_id)
    }

    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError> {
        self.engine
            .cancel(broker_order_id)
            .map_err(|_| BrokerError::OrderNotFound(broker_order_id.to_owned()))
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        self.engine
            .order_status(broker_order_id)
            .ok_or_else(|| BrokerError::OrderNotFound(broker_order_id.to_owned()))
        // Paper orders fill immediately on submit; nothing is pending afterward.
        Err(BrokerError::OrderNotFound(broker_order_id.to_owned()))
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        Ok(self.engine.open_orders(Some(self.asset_class)))
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        Ok(self.engine.positions(self.asset_class))
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::BrokerOrderState;
    use domain::{
        money::{Price, Size},
        order::{OrderIntent, OrderType, Side},
    };
    use rust_decimal_macros::dec;

    fn make_market_intent(instrument_id: &str) -> OrderIntent {
        OrderIntent::new(
            instrument_id,
            Side::Buy,
            OrderType::Market,
            Size::from_decimal(dec!(1)),
            None,
            None,
        )
    }

    #[tokio::test]
    async fn paper_broker_fills_immediately_and_tracks_position() {
        let (broker, engine) = PaperBroker::new(AssetClass::CryptoSpotCex);
        engine.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));

        let intent = make_market_intent("BTC-USD");
        let approved = risk::ApprovedOrder::new_for_test(intent);
        let order_id = broker.submit(&approved).await.expect("paper submit");

        let status = broker.query_order(&order_id).await.expect("queryable");
        assert_eq!(status.state, BrokerOrderState::Filled);

        let positions = broker.query_positions().await.unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].quantity, dec!(1));
    }

    #[tokio::test]
    async fn paper_broker_rejects_without_mark_price() {
        let (broker, _engine) = PaperBroker::new(AssetClass::Equity);
        let approved = risk::ApprovedOrder::new_for_test(make_market_intent("AAPL"));
        let result = broker.submit(&approved).await;
        assert!(matches!(result, Err(BrokerError::Rejected(_))));
    }

    #[tokio::test]
    async fn paper_broker_cancel_unknown_order_errors() {
        let (broker, _engine) = PaperBroker::new(AssetClass::Equity);
        assert!(broker.cancel("missing").await.is_err());
    }

    #[tokio::test]
    async fn paper_broker_lists_resting_limit_as_open() {
        let (broker, engine) = PaperBroker::new(AssetClass::CryptoSpotCex);
        engine.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));

        let intent = OrderIntent::new(
            "BTC-USD",
            Side::Buy,
            OrderType::Limit,
            Size::from_decimal(dec!(1)),
            Some(Price::from_decimal(dec!(45_000))),
            None,
        );
        let approved = risk::ApprovedOrder::new_for_test(intent);
        let order_id = broker.submit(&approved).await.unwrap();

        let open = broker.query_open_orders().await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].broker_order_id, order_id);

        broker.cancel(&order_id).await.unwrap();
        assert!(broker.query_open_orders().await.unwrap().is_empty());
    }
}
