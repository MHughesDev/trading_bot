//! Sacred event publication for order / position / balance lifecycle.
//!
//! These lanes use never-drop delivery policies.  Do not treat them like
//! UI fanout — every consumer must receive every message.

/// Lane names for sacred execution events.
pub mod lanes {
    pub const ORDERS_ACCEPTED: &str = "orders.accepted";
    pub const ORDERS_REJECTED: &str = "orders.rejected";
    pub const ORDERS_SUBMITTED: &str = "orders.submitted";
    pub const ORDERS_CANCEL_REQUESTED: &str = "orders.cancel_requested";
    pub const ORDERS_CANCELLED: &str = "orders.cancelled";
    pub const ORDERS_PARTIALLY_FILLED: &str = "orders.partially_filled";
    pub const ORDERS_FILLED: &str = "orders.filled";
    pub const POSITIONS_UPDATED: &str = "positions.updated";
    pub const BALANCES_UPDATED: &str = "balances.updated";
}
