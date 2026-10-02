//! Durable record of every order and every fill.
//!
//! Before this module the repository contained **no** `INSERT INTO orders` and
//! no `INSERT INTO fills`.  `GET /api/orders/{id}` read a table nothing ever
//! wrote, so it could not succeed.  Worse, `fills.rs` carried a comment saying
//! the `fills` UNIQUE constraint "provides the durable idempotency guarantee";
//! there was no such constraint and no such insert.  Idempotency was a 50 000
//! entry in-memory `VecDeque` that a restart wiped — which is precisely the
//! case SC-7 exists for, since a restart is a reconnect.
//!
//! What is durable now (migration 0121):
//!
//! * `orders.idempotency_key` is the primary key.  A resubmit of a key already
//!   recorded returns the stored broker order id **without calling the broker**,
//!   across process restarts.
//! * `fills` has `UNIQUE (idempotency_key, broker_order_id, fill_price,
//!   filled_size)`.  A redelivered fill inserts zero rows and is reported as a
//!   duplicate by the database, not by a cache.
//! * `orders.execution_mode` is `CHECK (execution_mode = 'paper')`.  Reaching a
//!   live venue is not a code review away; it is a migration away.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
// Re-exported: every key in this module's public API is one of these, so a
// caller can name the type without depending on `uuid` directly.
pub use uuid::Uuid;

use domain::{
    money::Price,
    order::{OrderIntent, OrderState, OrderType, Side},
};

/// The only execution mode this platform writes.  Paper, and the database
/// agrees (see the `CHECK` in migration 0121).
pub const PAPER_MODE: &str = "paper";

/// Errors from the journal.  A journal failure is never swallowed: an order
/// that cannot be recorded is an order that cannot be reconciled (FM-4).
#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("order journal database error: {0}")]
    Database(String),
    #[error("no order journal installed — refusing to submit an order that cannot be recorded")]
    NotInstalled,
}

/// What the journal knows about an order it has already seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedOrder {
    pub idempotency_key: Uuid,
    pub broker_order_id: Option<String>,
    pub state: OrderState,
}

/// A fill, in the shape it is persisted and published.
///
/// This is the record other components consume — reconciliation reads `fills`
/// rows with exactly these columns.
#[derive(Clone, Debug, PartialEq)]
pub struct PersistedFill {
    /// The order's idempotency key — joins to `orders.idempotency_key`.
    pub idempotency_key: Uuid,
    pub broker_order_id: String,
    pub instrument_id: String,
    pub side: Side,
    /// Quantity filled by **this** fill (the delta), which is what P&L uses.
    pub filled_size: Decimal,
    /// Cumulative quantity filled on the order **after** this fill.
    ///
    /// This is the field that makes a fill unique.  Keying on the delta alone
    /// (what the in-memory dedup does) collapses two legitimate partial fills
    /// of the same size at the same price into one — which would silently
    /// halve a position.  Cumulative quantity is monotonic, so two partials are
    /// distinct while a redelivery of the same sync is not.
    pub cumulative_filled: Decimal,
    pub fill_price: Price,
    /// Commission and venue fees, or `None` when the venue did not report
    /// them.  Stored as `NULL`, never as `0`: an unknown fee recorded as zero
    /// makes the fill look cheaper than it was, which is the one direction a
    /// cost error must never go.
    pub commission: Option<Decimal>,
    pub filled_at: DateTime<Utc>,
}

/// Whether a write was new or a redelivery the database already held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Inserted,
    Duplicate,
}

/// Durable order/fill record.
#[async_trait]
pub trait OrderJournal: Send + Sync {
    /// Look up an order by its idempotency key.  This is what makes a resubmit
    /// after a restart safe: the answer survives the process.
    async fn find_order(&self, key: Uuid) -> Result<Option<RecordedOrder>, JournalError>;

    /// Record an order that has passed the risk gate, before it reaches a
    /// broker.  `Duplicate` means this key was already recorded.
    async fn record_order(
        &self,
        intent: &OrderIntent,
        state: OrderState,
    ) -> Result<WriteOutcome, JournalError>;

    /// Attach the broker's order id and move the order's state.
    async fn set_broker_order(
        &self,
        key: Uuid,
        broker_order_id: &str,
        state: OrderState,
    ) -> Result<(), JournalError>;

    /// Move an order's state (a fill, a cancel, a rejection).
    async fn set_state(&self, key: Uuid, state: OrderState) -> Result<(), JournalError>;

    /// Record a fill.  `Duplicate` is the database refusing a redelivery, and
    /// is the durable half of SC-7.
    async fn record_fill(&self, fill: &PersistedFill) -> Result<WriteOutcome, JournalError>;

    /// Every fill for one order, oldest first.
    async fn fills_for(&self, key: Uuid) -> Result<Vec<PersistedFill>, JournalError>;
}

static JOURNAL: OnceLock<Arc<dyn OrderJournal>> = OnceLock::new();

/// Install the process-wide journal.  First install wins.
pub fn install(journal: Arc<dyn OrderJournal>) -> bool {
    JOURNAL.set(journal).is_ok()
}

/// The installed journal, if any.
#[must_use]
pub fn global() -> Option<Arc<dyn OrderJournal>> {
    JOURNAL.get().cloned()
}

/// The installed journal, or the refusal that says why there is none.
pub fn require() -> Result<Arc<dyn OrderJournal>, JournalError> {
    global().ok_or(JournalError::NotInstalled)
}

pub fn side_str(side: Side) -> &'static str {
    match side {
        Side::Buy => "buy",
        Side::Sell => "sell",
    }
}

pub fn order_type_str(t: OrderType) -> &'static str {
    t.as_str()
}

pub fn order_state_str(s: OrderState) -> &'static str {
    match s {
        OrderState::Accepted => "accepted",
        OrderState::Submitted => "submitted",
        OrderState::PartiallyFilled => "partially_filled",
        OrderState::Filled => "filled",
        OrderState::Cancelled => "cancelled",
        OrderState::Rejected => "rejected",
    }
}

/// Parse a persisted order state.  Unknown text is an error, never a default:
/// guessing a state is how an open order is treated as terminal.
pub fn order_state_from_str(s: &str) -> Result<OrderState, JournalError> {
    match s {
        "accepted" => Ok(OrderState::Accepted),
        "submitted" => Ok(OrderState::Submitted),
        "partially_filled" => Ok(OrderState::PartiallyFilled),
        "filled" => Ok(OrderState::Filled),
        "cancelled" => Ok(OrderState::Cancelled),
        "rejected" => Ok(OrderState::Rejected),
        other => Err(JournalError::Database(format!(
            "unknown order state '{other}' in the orders table"
        ))),
    }
}

// ── Postgres implementation ──────────────────────────────────────────────────

/// The production journal.
pub struct PgOrderJournal {
    pool: sqlx::PgPool,
}

impl PgOrderJournal {
    #[must_use]
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

fn db<E: std::fmt::Display>(e: E) -> JournalError {
    JournalError::Database(e.to_string())
}

#[async_trait]
impl OrderJournal for PgOrderJournal {
    async fn find_order(&self, key: Uuid) -> Result<Option<RecordedOrder>, JournalError> {
        let row: Option<(Option<String>, String)> =
            sqlx::query_as("SELECT broker_order_id, state FROM orders WHERE idempotency_key = $1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .map_err(db)?;
        match row {
            None => Ok(None),
            Some((broker_order_id, state)) => Ok(Some(RecordedOrder {
                idempotency_key: key,
                broker_order_id,
                state: order_state_from_str(&state)?,
            })),
        }
    }

    async fn record_order(
        &self,
        intent: &OrderIntent,
        state: OrderState,
    ) -> Result<WriteOutcome, JournalError> {
        let result = sqlx::query(
            "INSERT INTO orders \
               (idempotency_key, account_id, strategy_id, instrument_id, side, order_type, \
                size, limit_price, state, broker_order_id, execution_mode, created_at, updated_at) \
             VALUES ($1, NULL, $2, $3, $4, $5, $6, $7, $8, NULL, $9, $10, now()) \
             ON CONFLICT (idempotency_key) DO NOTHING",
        )
        .bind(intent.idempotency_key)
        .bind(intent.strategy_id.as_deref())
        .bind(&intent.instrument_id)
        .bind(side_str(intent.side))
        .bind(order_type_str(intent.order_type))
        .bind(intent.size.inner())
        .bind(intent.limit_price.map(|p| p.inner()))
        .bind(order_state_str(state))
        .bind(PAPER_MODE)
        .bind(intent.created_at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(if result.rows_affected() == 0 {
            WriteOutcome::Duplicate
        } else {
            WriteOutcome::Inserted
        })
    }

    async fn set_broker_order(
        &self,
        key: Uuid,
        broker_order_id: &str,
        state: OrderState,
    ) -> Result<(), JournalError> {
        sqlx::query(
            "UPDATE orders SET broker_order_id = $2, state = $3, updated_at = now() \
             WHERE idempotency_key = $1",
        )
        .bind(key)
        .bind(broker_order_id)
        .bind(order_state_str(state))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn set_state(&self, key: Uuid, state: OrderState) -> Result<(), JournalError> {
        sqlx::query("UPDATE orders SET state = $2, updated_at = now() WHERE idempotency_key = $1")
            .bind(key)
            .bind(order_state_str(state))
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn record_fill(&self, fill: &PersistedFill) -> Result<WriteOutcome, JournalError> {
        let result = sqlx::query(
            "INSERT INTO fills \
               (idempotency_key, broker_order_id, instrument_id, side, filled_size, \
                cumulative_filled, fill_price, commission, filled_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT ON CONSTRAINT fills_idempotent DO NOTHING",
        )
        .bind(fill.idempotency_key)
        .bind(&fill.broker_order_id)
        .bind(&fill.instrument_id)
        .bind(side_str(fill.side))
        .bind(fill.filled_size)
        .bind(fill.cumulative_filled)
        .bind(fill.fill_price.inner())
        .bind(fill.commission)
        .bind(fill.filled_at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(if result.rows_affected() == 0 {
            WriteOutcome::Duplicate
        } else {
            WriteOutcome::Inserted
        })
    }

    async fn fills_for(&self, key: Uuid) -> Result<Vec<PersistedFill>, JournalError> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            String,
            String,
            String,
            Decimal,
            Decimal,
            Decimal,
            Option<Decimal>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT broker_order_id, instrument_id, side, filled_size, cumulative_filled, \
                        fill_price, commission, filled_at \
                 FROM fills WHERE idempotency_key = $1 ORDER BY filled_at ASC",
        )
        .bind(key)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.into_iter()
            .map(
                |(
                    broker_order_id,
                    instrument_id,
                    side,
                    filled_size,
                    cumulative_filled,
                    fill_price,
                    commission,
                    at,
                )| {
                    Ok(PersistedFill {
                        idempotency_key: key,
                        broker_order_id,
                        instrument_id,
                        side: parse_side(&side)?,
                        filled_size,
                        cumulative_filled,
                        fill_price: Price::from_decimal(fill_price),
                        commission,
                        filled_at: at,
                    })
                },
            )
            .collect()
    }
}

/// Parse a persisted side.  An unrecognised value is an error.
///
/// The predecessor of this function read `if s == "buy" { Buy } else { Sell }`,
/// so any unexpected text — a typo, a truncation, a new venue's spelling —
/// became a **sell**.  A malformed input silently becoming a position-reducing
/// or short-opening order is the exact inverse of this track's rule: when an
/// input cannot be trusted, refuse.
pub fn parse_side(s: &str) -> Result<Side, JournalError> {
    match s {
        "buy" => Ok(Side::Buy),
        "sell" => Ok(Side::Sell),
        other => Err(JournalError::Database(format!(
            "unknown order side '{other}' — refusing to guess a direction"
        ))),
    }
}

/// Parse an order type.  Unknown text is an error, never `Market`.
pub fn parse_order_type(s: &str) -> Result<OrderType, JournalError> {
    match s {
        "market" => Ok(OrderType::Market),
        "limit" => Ok(OrderType::Limit),
        "stop" => Ok(OrderType::Stop),
        "stop_limit" => Ok(OrderType::StopLimit),
        "trailing_stop" => Ok(OrderType::TrailingStop),
        other => Err(JournalError::Database(format!(
            "unknown order type '{other}' — refusing to guess"
        ))),
    }
}

// ── In-memory journal, for tests ─────────────────────────────────────────────

/// A non-durable journal with the same semantics, for tests that have no
/// database.  Deliberately feature-gated: installing this in a process that
/// trades would reinstate exactly the in-memory idempotency this module exists
/// to replace.
#[cfg(any(test, feature = "test-helpers"))]
pub mod memory {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Inner {
        orders: HashMap<Uuid, RecordedOrder>,
        // (key, broker_order_id, price, size) → fill
        fills: Vec<(String, PersistedFill)>,
    }

    #[derive(Default)]
    pub struct MemoryOrderJournal {
        inner: Mutex<Inner>,
    }

    impl MemoryOrderJournal {
        #[must_use]
        pub fn new() -> Self {
            Self::default()
        }
    }

    /// Mirrors the `fills_idempotent` constraint exactly, cumulative quantity
    /// included — a test journal that deduped differently from the database
    /// would prove the wrong thing.
    fn dedup_key(f: &PersistedFill) -> String {
        format!(
            "{}|{}|{}",
            f.idempotency_key, f.broker_order_id, f.cumulative_filled
        )
    }

    #[async_trait]
    impl OrderJournal for MemoryOrderJournal {
        async fn find_order(&self, key: Uuid) -> Result<Option<RecordedOrder>, JournalError> {
            Ok(self.inner.lock().unwrap().orders.get(&key).cloned())
        }

        async fn record_order(
            &self,
            intent: &OrderIntent,
            state: OrderState,
        ) -> Result<WriteOutcome, JournalError> {
            let mut inner = self.inner.lock().unwrap();
            if inner.orders.contains_key(&intent.idempotency_key) {
                return Ok(WriteOutcome::Duplicate);
            }
            inner.orders.insert(
                intent.idempotency_key,
                RecordedOrder {
                    idempotency_key: intent.idempotency_key,
                    broker_order_id: None,
                    state,
                },
            );
            Ok(WriteOutcome::Inserted)
        }

        async fn set_broker_order(
            &self,
            key: Uuid,
            broker_order_id: &str,
            state: OrderState,
        ) -> Result<(), JournalError> {
            let mut inner = self.inner.lock().unwrap();
            if let Some(o) = inner.orders.get_mut(&key) {
                o.broker_order_id = Some(broker_order_id.to_owned());
                o.state = state;
            }
            Ok(())
        }

        async fn set_state(&self, key: Uuid, state: OrderState) -> Result<(), JournalError> {
            let mut inner = self.inner.lock().unwrap();
            if let Some(o) = inner.orders.get_mut(&key) {
                o.state = state;
            }
            Ok(())
        }

        async fn record_fill(&self, fill: &PersistedFill) -> Result<WriteOutcome, JournalError> {
            let mut inner = self.inner.lock().unwrap();
            let k = dedup_key(fill);
            if inner.fills.iter().any(|(existing, _)| existing == &k) {
                return Ok(WriteOutcome::Duplicate);
            }
            inner.fills.push((k, fill.clone()));
            Ok(WriteOutcome::Inserted)
        }

        async fn fills_for(&self, key: Uuid) -> Result<Vec<PersistedFill>, JournalError> {
            Ok(self
                .inner
                .lock()
                .unwrap()
                .fills
                .iter()
                .filter(|(_, f)| f.idempotency_key == key)
                .map(|(_, f)| f.clone())
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_side_is_refused_not_defaulted_to_sell() {
        for bad in ["", "SELL ", "b", "short", "BUY", "sel"] {
            assert!(
                parse_side(bad).is_err(),
                "side '{bad}' must be refused, not read as a sell"
            );
        }
        assert_eq!(parse_side("buy").unwrap(), Side::Buy);
        assert_eq!(parse_side("sell").unwrap(), Side::Sell);
    }

    #[test]
    fn unknown_order_type_is_refused_not_defaulted_to_market() {
        // "stop" was the unknown example until 2026-10-02, when stop and
        // trailing-stop orders joined the vocabulary (Set U U-2.3).
        for bad in ["", "STOP", "stop_market", "trailing", "mkt"] {
            assert!(parse_order_type(bad).is_err(), "'{bad}' must be refused");
        }
        for t in [
            OrderType::Market,
            OrderType::Limit,
            OrderType::Stop,
            OrderType::StopLimit,
            OrderType::TrailingStop,
        ] {
            assert_eq!(parse_order_type(order_type_str(t)).unwrap(), t);
        }
    }

    #[test]
    fn unknown_order_state_is_refused() {
        assert!(order_state_from_str("weird").is_err());
        assert_eq!(order_state_from_str("filled").unwrap(), OrderState::Filled);
    }

    #[test]
    fn state_strings_round_trip() {
        for s in [
            OrderState::Accepted,
            OrderState::Submitted,
            OrderState::PartiallyFilled,
            OrderState::Filled,
            OrderState::Cancelled,
            OrderState::Rejected,
        ] {
            assert_eq!(order_state_from_str(order_state_str(s)).unwrap(), s);
        }
    }
}
