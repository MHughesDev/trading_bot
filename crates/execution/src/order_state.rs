//! Order lifecycle state machine.
//!
//! Transitions:
//!   None → Accepted → Submitted → PartiallyFilled → Filled
//!                               → Cancelled
//!                               → Rejected
//! Late arrivals and invalid transitions are rejected with an error.

use chrono::{DateTime, Utc};
use domain::order::OrderState;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("invalid state transition from {from:?} to {to:?} for order {order_id}")]
    InvalidTransition {
        order_id: Uuid,
        from: OrderState,
        to: OrderState,
    },
}

/// Returns `Ok(new_state)` if `transition` is valid from `current`, or an error.
pub fn transition(
    order_id: Uuid,
    current: OrderState,
    to: OrderState,
) -> Result<OrderState, StateError> {
    let allowed = match current {
        OrderState::Accepted => matches!(
            to,
            OrderState::Submitted | OrderState::Rejected | OrderState::Cancelled
        ),
        OrderState::Submitted => matches!(
            to,
            OrderState::PartiallyFilled
                | OrderState::Filled
                | OrderState::Cancelled
                | OrderState::Rejected
        ),
        OrderState::PartiallyFilled => matches!(
            to,
            OrderState::Filled | OrderState::Cancelled | OrderState::PartiallyFilled
        ),
        // Terminal states may not transition further.
        OrderState::Filled | OrderState::Cancelled | OrderState::Rejected => false,
    };

    if allowed {
        Ok(to)
    } else {
        Err(StateError::InvalidTransition {
            order_id,
            from: current,
            to,
        })
    }
}

/// A snapshot of an order's current lifecycle state.
#[derive(Debug, Clone)]
pub struct OrderRecord {
    pub idempotency_key: Uuid,
    pub broker_order_id: Option<String>,
    pub instrument_id: String,
    pub state: OrderState,
    pub submitted_qty: rust_decimal::Decimal,
    pub filled_qty: rust_decimal::Decimal,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_can_submit() {
        assert!(transition(Uuid::new_v4(), OrderState::Accepted, OrderState::Submitted).is_ok());
    }

    #[test]
    fn accepted_can_reject() {
        assert!(transition(Uuid::new_v4(), OrderState::Accepted, OrderState::Rejected).is_ok());
    }

    #[test]
    fn filled_is_terminal() {
        let err =
            transition(Uuid::new_v4(), OrderState::Filled, OrderState::Submitted).unwrap_err();
        assert!(matches!(err, StateError::InvalidTransition { .. }));
    }

    #[test]
    fn submitted_to_partial_to_filled() {
        let id = Uuid::new_v4();
        let s1 = transition(id, OrderState::Submitted, OrderState::PartiallyFilled).unwrap();
        let s2 = transition(id, s1, OrderState::Filled).unwrap();
        assert_eq!(s2, OrderState::Filled);
    }
}
