//! Execution engine — order state machine, broker adapters, fills, positions.

pub mod account;
pub mod account_source;
pub mod alpaca;
pub mod audit;
pub mod broker;
pub mod coinbase;
pub mod events;
pub mod fills;
pub mod journal;
pub mod order_state;
pub mod paper;
pub mod positions;
pub mod venues;

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
    /// `true` when this key was already recorded in the journal and the broker
    /// was **not** called again.  A redelivery, a reconnect, or a resubmit
    /// after a restart all land here (SC-7).
    pub duplicate: bool,
    /// The fill captured during this submission, if the venue filled it
    /// immediately (paper market orders do).  `None` for a resting order.
    pub fill: Option<journal::PersistedFill>,
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

    /// Submit an `ApprovedOrder` to the broker, recording it durably first.
    ///
    /// Order of operations matters and is not negotiable:
    ///
    /// 1. **Ask the journal whether this key was already submitted.** If it
    ///    was, return the recorded broker order id without touching the broker.
    ///    This is what makes a redelivery, a reconnect and a *restart* safe;
    ///    the in-memory caches above cannot do it because a restart wipes them.
    /// 2. **Record the order before submitting it.** If the process dies
    ///    between the record and the ack, the order exists in the journal with
    ///    no broker id, which reconciliation can see and query.  Recording
    ///    afterwards would lose orders in exactly the window that matters.
    /// 3. Submit, then attach the broker id.
    /// 4. Query the order back and record any fill.
    ///
    /// An order that cannot be recorded is not submitted: a trade the platform
    /// cannot reconcile is worse than a trade it did not make (FM-4).
    pub async fn submit(&self, order: ApprovedOrder) -> Result<SubmitResult, ExecutionError> {
        let idempotency_key = order.intent.idempotency_key;
        let journal = journal::require().map_err(|e| ExecutionError::Database(e.to_string()))?;

        // 1. Durable idempotency check.
        if let Some(recorded) = journal
            .find_order(idempotency_key)
            .await
            .map_err(|e| ExecutionError::Database(e.to_string()))?
        {
            if let Some(broker_order_id) = recorded.broker_order_id {
                info!(
                    %idempotency_key,
                    %broker_order_id,
                    "order already submitted — returning the recorded broker id without resubmitting"
                );
                let fill = journal
                    .fills_for(idempotency_key)
                    .await
                    .map_err(|e| ExecutionError::Database(e.to_string()))?
                    .into_iter()
                    .next_back();
                return Ok(SubmitResult {
                    broker_order_id,
                    audit: vec![AuditEntry::broker_query(
                        idempotency_key,
                        "duplicate-submit".to_owned(),
                        "idempotent-replay".to_owned(),
                    )],
                    duplicate: true,
                    fill,
                });
            }
            // Recorded, but no broker id.  Two different situations, and
            // conflating them would either hide a rejection or invite a
            // double-fill.
            return Err(match recorded.state {
                // The venue refused it.  COMP-002 §3.4: a failed order is
                // retry-eligible **with a new idempotency key** — reusing the
                // key would collide with this row for ever.
                OrderState::Rejected => ExecutionError::Database(format!(
                    "order {idempotency_key} was rejected by the venue; \
                     retry with a new idempotency key (COMP-002 §3.4)"
                )),
                // Recorded and never acked: an ack desync.  COMP-002 §3.6 —
                // query, never blindly retry.  There is no broker id to query
                // with, so the safe answer is to refuse and let reconciliation
                // resolve it rather than risk a double-fill.
                _ => ExecutionError::Database(format!(
                    "order {idempotency_key} was recorded but has no broker ack — \
                     refusing to resubmit blindly (COMP-002 §3.6 ack desync)"
                )),
            });
        }

        // 2. Record before submitting.
        journal
            .record_order(&order.intent, OrderState::Accepted)
            .await
            .map_err(|e| ExecutionError::Database(e.to_string()))?;

        // 3. Submit.  A venue refusal is recorded as a refusal: leaving the row
        //    at `accepted` would make a rejected order indistinguishable from
        //    an order whose ack went missing, and those need opposite responses.
        let broker_order_id = match self.broker.submit(&order).await {
            Ok(id) => id,
            Err(e) => {
                if let Err(je) = journal.set_state(idempotency_key, OrderState::Rejected).await {
                    tracing::error!(
                        %idempotency_key, error = %je,
                        "could not record a venue rejection — the order row is left at 'accepted' \
                         and will read as an ack desync"
                    );
                }
                return Err(e.into());
            }
        };
        journal
            .set_broker_order(idempotency_key, &broker_order_id, OrderState::Submitted)
            .await
            .map_err(|e| ExecutionError::Database(e.to_string()))?;

        let mut audit = vec![
            AuditEntry::state_transition(
                idempotency_key,
                Some(OrderState::Accepted),
                OrderState::Submitted,
                None,
            ),
            AuditEntry::broker_submit(idempotency_key, broker_order_id.clone()),
        ];

        info!(%idempotency_key, %broker_order_id, "order submitted");

        // 4. Capture the fill.  Paper market orders fill on submit, so without
        //    this step the platform would have an order and never a fill —
        //    which is the state Gate 15 cannot start a clock against.
        let fill = self
            .capture_fill(
                &*journal,
                idempotency_key,
                &broker_order_id,
                &order.intent,
                &mut audit,
            )
            .await?;

        Ok(SubmitResult {
            broker_order_id,
            audit,
            duplicate: false,
            fill,
        })
    }

    /// Query the order back from the venue and persist whatever filled.
    async fn capture_fill(
        &self,
        journal: &dyn journal::OrderJournal,
        idempotency_key: Uuid,
        broker_order_id: &str,
        intent: &OrderIntent,
        audit: &mut Vec<AuditEntry>,
    ) -> Result<Option<journal::PersistedFill>, ExecutionError> {
        let status = match self.broker.query_order(broker_order_id).await {
            Ok(s) => s,
            Err(e) => {
                // The order is submitted and recorded; the fill will be picked
                // up by the reconciliation sweep.  Not an error for the caller.
                tracing::warn!(
                    %idempotency_key, %broker_order_id, error = %e,
                    "could not read the order back after submit — leaving the fill to reconciliation"
                );
                return Ok(None);
            }
        };

        if status.filled_qty <= Decimal::ZERO {
            return Ok(None);
        }
        let Some(avg_price) = status.avg_fill_price else {
            tracing::warn!(
                %idempotency_key, %broker_order_id,
                "venue reports a filled quantity with no fill price — not recording a fill"
            );
            return Ok(None);
        };

        let fill = journal::PersistedFill {
            idempotency_key,
            broker_order_id: broker_order_id.to_owned(),
            instrument_id: intent.instrument_id.clone(),
            side: intent.side,
            filled_size: status.filled_qty,
            fill_price: avg_price,
            commission: status.commission,
            filled_at: Utc::now(),
        };
        if fill.commission.is_none() {
            tracing::warn!(
                %idempotency_key, %broker_order_id,
                "venue did not report a commission for this fill \u2014 stored as NULL, not as \
                 zero. P&L from this fill is gross of fees until the adapter reports them."
            );
        }

        // The event shape every consumer of the fill stream receives.
        //
        // `FillEvent::commission` is a plain `Decimal` and cannot express
        // "unknown", so an unreported fee appears here as zero.  The
        // authoritative test is `fills.commission IS NULL` in the journal; the
        // warning above fires whenever the two differ.
        let event = FillEvent {
            idempotency_key,
            broker_order_id: broker_order_id.to_owned(),
            filled_qty: fill.filled_size,
            fill_price: fill.fill_price,
            commission: fill.commission.unwrap_or(Decimal::ZERO),
            filled_at: fill.filled_at,
        };

        // In-memory dedup first (cheap), database second (durable).
        let in_memory = self
            .fill_processor
            .lock()
            .map(|mut fp| fp.apply(&event))
            .unwrap_or(FillResult::Applied);

        let outcome = journal
            .record_fill(&fill)
            .await
            .map_err(|e| ExecutionError::Database(e.to_string()))?;

        let new_state = if status.filled_qty >= status.submitted_qty {
            OrderState::Filled
        } else {
            OrderState::PartiallyFilled
        };
        journal
            .set_state(idempotency_key, new_state)
            .await
            .map_err(|e| ExecutionError::Database(e.to_string()))?;

        if outcome == journal::WriteOutcome::Inserted && in_memory == FillResult::Applied {
            audit.push(AuditEntry::StateTransition {
                idempotency_key,
                from: Some(OrderState::Submitted),
                to: new_state,
                reason: None,
                at: Utc::now(),
            });
            info!(
                %idempotency_key, %broker_order_id,
                filled = %fill.filled_size, price = %fill.fill_price.inner(),
                observers = fills::observer_count(),
                "fill recorded"
            );

            // The fill stream.  Published only for a fill that is new to both
            // the in-memory processor and the database — a redelivery or a
            // post-restart resubmit must not look like a second forward
            // observation to reconciliation or to Gate 15.
            fills::publish(&event);
        } else {
            tracing::debug!(
                %idempotency_key,
                in_memory = ?in_memory,
                "fill was already recorded — durable idempotency held"
            );
        }

        Ok(Some(fill))
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

/// Parse a persisted `Side`.  `None` for anything unrecognised.
///
/// This used to read `if s == "buy" { Buy } else { Sell }`, so a malformed,
/// truncated or unexpected value silently became a **sell** — a
/// position-reducing or short-opening order built from an input the platform
/// could not read.  When an input cannot be trusted, refuse.
///
/// [`journal::parse_side`] is the same check with a reason attached; prefer it
/// where an error can be surfaced.
#[must_use]
pub fn side_from_str(s: &str) -> Option<Side> {
    journal::parse_side(s).ok()
}

/// Parse a persisted `OrderType`.  `None` for anything unrecognised — an
/// unknown type must never fall through to `Market`, which is the one type
/// that executes immediately at any price.
#[must_use]
pub fn order_type_from_str(s: &str) -> Option<OrderType> {
    journal::parse_order_type(s).ok()
}

pub fn decimal_or_zero(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or(Decimal::ZERO)
}

/// Mock broker available for integration tests in dependent crates.
pub mod mock_broker;
