//! Append-only execution audit trail.
//!
//! Every state transition and fill is written as an immutable audit record.
//! Records are never updated in place — corrections are new entries.

use chrono::{DateTime, Utc};
use domain::order::OrderState;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An entry in the immutable audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditEntry {
    /// Order state changed.
    StateTransition {
        idempotency_key: Uuid,
        from: Option<OrderState>,
        to: OrderState,
        reason: Option<String>,
        at: DateTime<Utc>,
    },
    /// A fill (partial or full) was received and processed.
    FillReceived {
        idempotency_key: Uuid,
        broker_order_id: String,
        filled_qty: Decimal,
        fill_price: Decimal,
        commission: Decimal,
        at: DateTime<Utc>,
    },
    /// Order submitted to broker; broker ID recorded.
    BrokerSubmit {
        idempotency_key: Uuid,
        broker_order_id: String,
        at: DateTime<Utc>,
    },
    /// Query sent to broker (e.g. on missing ack).
    BrokerQuery {
        idempotency_key: Uuid,
        broker_order_id: String,
        reason: String,
        at: DateTime<Utc>,
    },
}

impl AuditEntry {
    pub fn state_transition(
        idempotency_key: Uuid,
        from: Option<OrderState>,
        to: OrderState,
        reason: Option<String>,
    ) -> Self {
        Self::StateTransition {
            idempotency_key,
            from,
            to,
            reason,
            at: Utc::now(),
        }
    }

    pub fn broker_submit(idempotency_key: Uuid, broker_order_id: String) -> Self {
        Self::BrokerSubmit {
            idempotency_key,
            broker_order_id,
            at: Utc::now(),
        }
    }

    pub fn broker_query(idempotency_key: Uuid, broker_order_id: String, reason: String) -> Self {
        Self::BrokerQuery {
            idempotency_key,
            broker_order_id,
            reason,
            at: Utc::now(),
        }
    }
}
