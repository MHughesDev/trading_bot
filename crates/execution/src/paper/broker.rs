//! In-house paper broker — implements `Broker` using the local fill simulators.
//!
//! No external API calls are ever made. Fills are simulated immediately at the
//! latest mark price. This is the default execution path for all accounts that
//! have not loaded live trading credentials from the database.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use domain::{instrument::AssetClass, money::Price};
use risk::ApprovedOrder;
use rust_decimal::Decimal;
use tracing::info;
use uuid::Uuid;

use super::simulator_for;
use crate::broker::{Broker, BrokerError, BrokerOrderStatus, BrokerPosition};

/// In-house paper fill broker for a single asset class.
///
/// The current mark price is shared via `Arc<RwLock<Price>>` so the hot-path
/// bar-builder stage can update it on every tick without any allocation.
/// Live trading credentials are never read here — all fills are simulated locally.
pub struct PaperBroker {
    mark: Arc<RwLock<Price>>,
    asset_class: AssetClass,
}

impl PaperBroker {
    /// Create a paper broker and return the shared mark-price handle.
    ///
    /// The caller should keep the returned `Arc<RwLock<Price>>` and write
    /// fresh prices into it as ticks arrive. The broker reads it on each `submit`.
    pub fn new(asset_class: AssetClass) -> (Self, Arc<RwLock<Price>>) {
        let mark = Arc::new(RwLock::new(Price::from_decimal(Decimal::ONE)));
        let broker = Self {
            mark: Arc::clone(&mark),
            asset_class,
        };
        (broker, mark)
    }
}

#[async_trait]
impl Broker for PaperBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let mark = *self.mark.read().expect("mark lock poisoned");
        let simulator = simulator_for(self.asset_class.market_structure());
        let fill = simulator.simulate_fill(&order.intent, mark);

        info!(
            instrument_id = %order.intent.instrument_id,
            side          = ?order.intent.side,
            fill_price    = %fill.fill_price,
            filled_qty    = %fill.filled_qty,
            fee           = %fill.fee,
            "paper fill simulated (in-house)"
        );

        Ok(Uuid::new_v4().to_string())
    }

    async fn cancel(&self, _broker_order_id: &str) -> Result<(), BrokerError> {
        Ok(())
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        // Paper orders fill immediately on submit; nothing is pending afterward.
        Err(BrokerError::OrderNotFound(broker_order_id.to_owned()))
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        Ok(vec![])
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        Ok(vec![])
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{
        instrument::AssetClass,
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
    async fn paper_broker_fills_immediately() {
        let (broker, mark_handle) = PaperBroker::new(AssetClass::CryptoSpotCex);
        *mark_handle.write().unwrap() = Price::from_decimal(dec!(50000));

        let intent = make_market_intent("BTC-USD");
        let approved = risk::ApprovedOrder::new_for_test(intent);
        let result = broker.submit(&approved).await;
        assert!(result.is_ok(), "paper submit should succeed: {result:?}");
    }

    #[tokio::test]
    async fn paper_broker_cancel_is_noop() {
        let (broker, _) = PaperBroker::new(AssetClass::Equity);
        assert!(broker.cancel("any-id").await.is_ok());
    }

    #[tokio::test]
    async fn paper_broker_no_open_orders() {
        let (broker, _) = PaperBroker::new(AssetClass::Equity);
        let orders = broker.query_open_orders().await.unwrap();
        assert!(orders.is_empty());
    }
}
