//! Multi-asset paper broker — the "paper half" execution entry point.
//!
//! Live and paper share the same collector data: every pipeline feeds marks
//! into the one [`PaperTradingEngine`] and registers its instrument's asset
//! class.  Execution then splits by mode — live orders go to venue broker
//! adapters (via `venue-router`), paper orders come here, where each order
//! is routed to the internal account of **its instrument's** asset class.
//! This replaces wiring a single-class [`super::PaperBroker`] into the
//! execution engine (which sent every order to one account).

use std::sync::Arc;

use async_trait::async_trait;
use tracing::info;

use risk::ApprovedOrder;

use crate::broker::{Broker, BrokerError, BrokerOrderStatus, BrokerPosition};

use super::engine::PaperTradingEngine;

/// `Broker` implementation that resolves the asset class per order from the
/// engine's instrument registry.
#[derive(Clone)]
pub struct MultiAssetPaperBroker {
    engine: Arc<PaperTradingEngine>,
}

impl MultiAssetPaperBroker {
    /// Broker view over an existing engine
    /// (see [`PaperTradingEngine::multi_asset_broker`]).
    pub fn for_engine(engine: Arc<PaperTradingEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Broker for MultiAssetPaperBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        let instrument_id = &order.intent.instrument_id;
        let asset_class = self.engine.asset_class_of(instrument_id).ok_or_else(|| {
            BrokerError::Rejected(format!(
                "no asset class registered for instrument {instrument_id}"
            ))
        })?;

        let order_id = self
            .engine
            .submit(asset_class, &order.intent)
            .map_err(|e| BrokerError::Rejected(e.to_string()))?;

        if let Some(status) = self.engine.order_status(&order_id) {
            info!(
                %order_id,
                %instrument_id,
                asset_class = asset_class.as_str(),
                state = ?status.state,
                filled_qty = %status.filled_qty,
                "paper order routed to asset-class account"
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
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        Ok(self.engine.open_orders(None))
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        // All positions across every asset-class account.
        let mut out = Vec::new();
        for ac in super::policy::ALL_ASSET_CLASSES {
            out.extend(self.engine.positions(ac));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::instrument::AssetClass;
    use domain::money::{Price, Size};
    use domain::order::{OrderIntent, OrderType, Side};
    use rust_decimal_macros::dec;

    fn market(instrument: &str) -> OrderIntent {
        OrderIntent::new(
            instrument,
            Side::Buy,
            OrderType::Market,
            Size::from_decimal(dec!(1)),
            None,
            None,
        )
    }

    #[tokio::test]
    async fn orders_route_to_their_instruments_asset_class_account() {
        let engine = Arc::new(PaperTradingEngine::new());
        engine.register_instrument("BTC-USD", AssetClass::CryptoSpotCex);
        engine.register_instrument("EUR-USD", AssetClass::Fx);
        engine.on_mark("BTC-USD", Price::from_decimal(dec!(50_000)));
        engine.on_mark("EUR-USD", Price::from_decimal(dec!(1.10)));

        let broker = engine.multi_asset_broker();
        broker
            .submit(&risk::ApprovedOrder::new_for_test(market("BTC-USD")))
            .await
            .unwrap();
        broker
            .submit(&risk::ApprovedOrder::new_for_test(market("EUR-USD")))
            .await
            .unwrap();

        // Each fill landed in its own asset-class account.
        assert_eq!(engine.positions(AssetClass::CryptoSpotCex).len(), 1);
        assert_eq!(engine.positions(AssetClass::Fx).len(), 1);
        assert!(engine.positions(AssetClass::Equity).is_empty());
    }

    #[tokio::test]
    async fn unregistered_instrument_is_rejected() {
        let engine = Arc::new(PaperTradingEngine::new());
        engine.on_mark("MYSTERY", Price::from_decimal(dec!(1)));
        let broker = engine.multi_asset_broker();
        let result = broker
            .submit(&risk::ApprovedOrder::new_for_test(market("MYSTERY")))
            .await;
        assert!(matches!(result, Err(BrokerError::Rejected(_))));
    }
}
