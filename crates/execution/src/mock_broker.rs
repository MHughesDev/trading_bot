//! Mock `Broker` for unit and integration tests.
//!
//! Behaviour is controlled by setting fields before use.

use async_trait::async_trait;
use rust_decimal::Decimal;
use std::sync::Mutex;

use domain::{
    money::Price,
    order::{OrderType, Side},
};
use risk::ApprovedOrder;

use crate::broker::{Broker, BrokerError, BrokerOrderState, BrokerOrderStatus, BrokerPosition};

/// Configurable mock that records calls and returns canned responses.
pub struct MockBroker {
    /// Positions the mock will return from `query_positions`.
    pub positions: Mutex<Vec<BrokerPosition>>,
    /// Whether `submit` should succeed (true) or reject (false).
    pub submit_succeeds: bool,
    /// Counter of `query_order` calls.
    pub query_calls: Mutex<u32>,
    /// Counter of `submit` calls.
    pub submit_calls: Mutex<u32>,
    /// The filled qty to report on `query_order`.
    pub filled_qty: Decimal,
    /// The fill price to report on `query_order`.
    pub fill_price: Option<Price>,
    /// The state to report on `query_order`.
    pub query_state: BrokerOrderState,
}

impl MockBroker {
    pub fn new() -> Self {
        Self {
            positions: Mutex::new(vec![]),
            submit_succeeds: true,
            query_calls: Mutex::new(0),
            submit_calls: Mutex::new(0),
            filled_qty: Decimal::ZERO,
            fill_price: None,
            query_state: BrokerOrderState::New,
        }
    }

    pub fn submit_call_count(&self) -> u32 {
        *self.submit_calls.lock().expect("submit_calls lock")
    }

    pub fn query_call_count(&self) -> u32 {
        *self.query_calls.lock().expect("query_calls lock")
    }
}

impl Default for MockBroker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Broker for MockBroker {
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError> {
        *self.submit_calls.lock().expect("submit_calls") += 1;
        if self.submit_succeeds {
            Ok(format!("mock-broker-{}", order.intent.idempotency_key))
        } else {
            Err(BrokerError::Rejected("mock rejection".to_owned()))
        }
    }

    async fn cancel(&self, _broker_order_id: &str) -> Result<(), BrokerError> {
        Ok(())
    }

    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError> {
        *self.query_calls.lock().expect("query_calls") += 1;
        Ok(BrokerOrderStatus {
            broker_order_id: broker_order_id.to_owned(),
            instrument_id: "BTC-USD".to_owned(),
            side: Side::Buy,
            order_type: OrderType::Market,
            submitted_qty: Decimal::from(1),
            filled_qty: self.filled_qty,
            avg_fill_price: self.fill_price,
            state: self.query_state.clone(),
        })
    }

    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError> {
        Ok(vec![])
    }

    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError> {
        Ok(self.positions.lock().expect("positions lock").clone())
    }
}
