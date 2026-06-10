//! `Broker` trait and associated types.
//!
//! Concrete implementations:
//! - `alpaca.rs` — Alpaca paper account (Phase 2)
//! - `coinbase.rs` — Coinbase live (post-Phase 6)
//!
//! The risk gate and strategy runtime never know which implementation is active.

use async_trait::async_trait;
use rust_decimal::Decimal;
use thiserror::Error;

use domain::{
    money::Price,
    order::{OrderType, Side},
};
use risk::ApprovedOrder;

/// Errors from broker interactions.
#[derive(Debug, Error)]
pub enum BrokerError {
    #[error("HTTP transport error: {0}")]
    Http(String),
    #[error("order not found: {0}")]
    OrderNotFound(String),
    #[error("order rejected by broker: {0}")]
    Rejected(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Summary of a broker-side order's current state.
#[derive(Debug, Clone)]
pub struct BrokerOrderStatus {
    pub broker_order_id: String,
    pub instrument_id: String,
    pub side: Side,
    pub order_type: OrderType,
    /// Quantity originally submitted.
    pub submitted_qty: Decimal,
    /// Cumulative quantity filled so far.
    pub filled_qty: Decimal,
    /// Volume-weighted average fill price (None if unfilled).
    pub avg_fill_price: Option<Price>,
    pub state: BrokerOrderState,
}

/// Simplified lifecycle states from the broker's perspective.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerOrderState {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Unknown(String),
}

/// A position as reported by the broker.
#[derive(Debug, Clone)]
pub struct BrokerPosition {
    pub instrument_id: String,
    /// Positive = long, negative = short.
    pub quantity: Decimal,
    pub avg_entry_price: Price,
}

/// Abstraction over execution venues.
///
/// All methods are `async` and return structured errors.  Missing acks must
/// be handled by querying (`query_order`) — never by blind retry.
#[async_trait]
pub trait Broker: Send + Sync {
    /// Submit an approved order.  Returns the broker-assigned order ID.
    async fn submit(&self, order: &ApprovedOrder) -> Result<String, BrokerError>;

    /// Cancel an open order by broker order ID.
    async fn cancel(&self, broker_order_id: &str) -> Result<(), BrokerError>;

    /// Query a single order by broker order ID.
    async fn query_order(&self, broker_order_id: &str) -> Result<BrokerOrderStatus, BrokerError>;

    /// Query all open orders.
    async fn query_open_orders(&self) -> Result<Vec<BrokerOrderStatus>, BrokerError>;

    /// Query current positions held at this broker.
    async fn query_positions(&self) -> Result<Vec<BrokerPosition>, BrokerError>;
}
