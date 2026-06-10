//! Sacred event publication for order / position / balance lifecycle.
//!
//! These lanes use never-drop delivery policies.  Do not treat them like
//! UI fanout — every consumer must receive every message.

use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::paper::PaperFill;
use storage::ledger::{AccountMode, FeePayload, FillPayload, FundingPaymentPayload, LedgerEvent};

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

/// Build a `LedgerEvent::Fill` from a paper fill.
///
/// The `usd_rate` is the USD/quote-currency exchange rate at fill time (1.0 for
/// USD-quoted instruments).
pub fn ledger_fill_from_paper(
    fill: &PaperFill,
    user_id: Uuid,
    account_mode: AccountMode,
    venue: &str,
    asset_class: &str,
    strategy_id: Option<Uuid>,
    usd_rate: Decimal,
) -> LedgerEvent {
    let id = Uuid::new_v4();
    let usd_value = fill.filled_qty * fill.fill_price.inner() * usd_rate;
    LedgerEvent::Fill {
        id,
        user_id,
        account_mode,
        venue: venue.to_owned(),
        asset_class: asset_class.to_owned(),
        instrument_id: fill.instrument_id.clone(),
        strategy_id,
        payload: FillPayload {
            side: format!("{:?}", fill.side).to_lowercase(),
            qty: fill.filled_qty,
            price: fill.fill_price.inner(),
            commission: fill.fee,
        },
        usd_value,
        context: serde_json::json!({
            "source": "paper_simulator",
            "idempotency_key": fill.idempotency_key.to_string(),
        }),
        occurred_at: Utc::now(),
    }
}

/// Build a `LedgerEvent::Fee` for an explicit fee event (e.g. platform fee).
#[allow(clippy::too_many_arguments)]
pub fn ledger_fee_event(
    user_id: Uuid,
    account_mode: AccountMode,
    venue: &str,
    asset_class: &str,
    instrument_id: &str,
    fee_type: &str,
    amount: Decimal,
    currency: &str,
    usd_rate: Decimal,
) -> LedgerEvent {
    LedgerEvent::Fee {
        id: Uuid::new_v4(),
        user_id,
        account_mode,
        venue: venue.to_owned(),
        asset_class: asset_class.to_owned(),
        instrument_id: instrument_id.to_owned(),
        strategy_id: None,
        payload: FeePayload {
            fee_type: fee_type.to_owned(),
            amount,
            currency: currency.to_owned(),
        },
        usd_value: amount * usd_rate,
        context: serde_json::json!({}),
        occurred_at: Utc::now(),
    }
}

/// Build a `LedgerEvent::FundingPayment` for a perpetual swap funding event.
#[allow(clippy::too_many_arguments)]
pub fn ledger_funding_event(
    user_id: Uuid,
    account_mode: AccountMode,
    venue: &str,
    instrument_id: &str,
    rate: Decimal,
    position_qty: Decimal,
    payment: Decimal,
    usd_rate: Decimal,
) -> LedgerEvent {
    LedgerEvent::FundingPayment {
        id: Uuid::new_v4(),
        user_id,
        account_mode,
        venue: venue.to_owned(),
        asset_class: "perpetual_swap".to_owned(),
        instrument_id: instrument_id.to_owned(),
        strategy_id: None,
        payload: FundingPaymentPayload {
            rate,
            position_qty,
            payment,
        },
        usd_value: payment * usd_rate,
        context: serde_json::json!({}),
        occurred_at: Utc::now(),
    }
}
