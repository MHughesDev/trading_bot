//! Execution engine — order state machine, broker adapters, fills, positions.

pub mod alpaca;
pub mod audit;
pub mod broker;
pub mod coinbase;
pub mod events;
pub mod fills;
pub mod order_state;
pub mod positions;

use std::sync::Arc;

use chrono::Utc;
use domain::order::{OrderIntent, OrderState, OrderType, Side};
use fills::{FillEvent, FillProcessor, FillResult};
use risk::ApprovedOrder;
use rust_decimal::Decimal;
use std::str::FromStr;
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

use crate::{
    audit::AuditEntry,
    broker::{Broker, BrokerError, BrokerOrderState},
    order_state::OrderRecord,
};

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("broker error: {0}")]
    Broker(#[from] BrokerError),
    #[error("order state error: {0}")]
    State(#[from] order_state::StateError),
    #[error("order not found: {0}")]
    NotFound(Uuid),
    #[error("database error: {0}")]
    Database(String),
}

/// Result of submitting an approved order to the execution engine.
pub struct SubmitResult {
    pub broker_order_id: String,
    pub audit: Vec<AuditEntry>,
}

/// Result of syncing a single order against the broker.
pub struct SyncResult {
    pub new_state: OrderState,
    pub fill: Option<FillEvent>,
    pub audit: Vec<AuditEntry>,
    pub fill_result: Option<FillResult>,
}

/// The execution engine.
///
/// Wraps a `Broker` implementation and manages the order lifecycle,
/// idempotent fill processing, and audit trail generation.
///
/// This struct is deliberately I/O-light in unit tests: the `Broker` trait is
/// satisfied by a mock, keeping the tests fast and deterministic.
pub struct ExecutionEngine {
    broker: Arc<dyn Broker>,
    fill_processor: std::sync::Mutex<FillProcessor>,
}

impl ExecutionEngine {
    pub fn new(broker: Arc<dyn Broker>) -> Self {
        Self {
            broker,
            fill_processor: std::sync::Mutex::new(FillProcessor::new()),
        }
    }

    /// Submit an `ApprovedOrder` to the broker and return the broker order ID.
    ///
    /// Idempotency is enforced by the broker (via `client_order_id` = idempotency key).
    /// A resubmit for an already-known key is a no-op at the broker level.
    pub async fn submit(&self, order: ApprovedOrder) -> Result<SubmitResult, ExecutionError> {
        let idempotency_key = order.intent.idempotency_key;

        let broker_order_id = self.broker.submit(&order).await?;

        let audit = vec![
            AuditEntry::state_transition(
                idempotency_key,
                Some(OrderState::Accepted),
                OrderState::Submitted,
                None,
            ),
            AuditEntry::broker_submit(idempotency_key, broker_order_id.clone()),
        ];

        info!(
            %idempotency_key,
            %broker_order_id,
            "order submitted"
        );

        Ok(SubmitResult {
            broker_order_id,
            audit,
        })
    }

    /// Cancel an open order by broker order ID.
    pub async fn cancel(&self, broker_order_id: &str) -> Result<(), ExecutionError> {
        self.broker.cancel(broker_order_id).await?;
        Ok(())
    }

    /// Query a single order by broker order ID and process any new fills.
    ///
    /// This is the **only** allowed response to a missing ack — never retry blindly.
    pub async fn sync_order(
        &self,
        idempotency_key: Uuid,
        broker_order_id: &str,
        prior_filled_qty: Decimal,
        _side: Side,
    ) -> Result<SyncResult, ExecutionError> {
        let status = self.broker.query_order(broker_order_id).await?;

        let mut audit = vec![AuditEntry::broker_query(
            idempotency_key,
            broker_order_id.to_owned(),
            "sync".to_owned(),
        )];

        let new_state = match status.state {
            BrokerOrderState::New => OrderState::Submitted,
            BrokerOrderState::PartiallyFilled => OrderState::PartiallyFilled,
            BrokerOrderState::Filled => OrderState::Filled,
            BrokerOrderState::Cancelled => OrderState::Cancelled,
            BrokerOrderState::Rejected => OrderState::Rejected,
            BrokerOrderState::Unknown(_) => OrderState::Submitted,
        };

        // Detect new fill delta.
        let delta = status.filled_qty - prior_filled_qty;
        let fill_and_result = if delta > Decimal::ZERO {
            if let Some(avg_price) = status.avg_fill_price {
                let fill = FillEvent {
                    idempotency_key,
                    broker_order_id: broker_order_id.to_owned(),
                    filled_qty: delta,
                    fill_price: avg_price,
                    commission: Decimal::ZERO,
                    filled_at: Utc::now(),
                };
                let result = self
                    .fill_processor
                    .lock()
                    .expect("fill_processor lock")
                    .apply(&fill);

                if result == FillResult::Applied {
                    audit.push(AuditEntry::StateTransition {
                        idempotency_key,
                        from: Some(OrderState::Submitted),
                        to: new_state,
                        reason: None,
                        at: Utc::now(),
                    });
                }

                Some((fill, result))
            } else {
                None
            }
        } else {
            None
        };

        let (fill, fill_result) = match fill_and_result {
            Some((f, r)) => (Some(f), Some(r)),
            None => (None, None),
        };

        Ok(SyncResult {
            new_state,
            fill,
            audit,
            fill_result,
        })
    }
}

// ── Helper for building an `OrderRecord` from an approved order ──────────────

pub fn order_record_from_intent(
    intent: &OrderIntent,
    broker_order_id: Option<String>,
) -> OrderRecord {
    OrderRecord {
        idempotency_key: intent.idempotency_key,
        broker_order_id,
        instrument_id: intent.instrument_id.clone(),
        state: OrderState::Accepted,
        submitted_qty: intent.size.inner(),
        filled_qty: Decimal::ZERO,
        created_at: intent.created_at,
        updated_at: Utc::now(),
    }
}

/// Determine the `Side` from an `OrderRecord` by looking up the original intent.
/// In tests this is constructed directly; in production it comes from the DB.
pub fn side_from_str(s: &str) -> Side {
    if s == "buy" {
        Side::Buy
    } else {
        Side::Sell
    }
}

pub fn order_type_from_str(s: &str) -> OrderType {
    match s {
        "limit" => OrderType::Limit,
        "stop_limit" => OrderType::StopLimit,
        _ => OrderType::Market,
    }
}

pub fn decimal_or_zero(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or(Decimal::ZERO)
}

/// Mock broker available for integration tests in dependent crates.
pub mod mock_broker;
