//! Idempotent fill processing.
//!
//! Fill processing is keyed by `(idempotency_key, broker_order_id, filled_qty, fill_price)`.
//! A replay of the same fill from JetStream is a no-op.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use domain::money::Price;

/// Maximum number of fill dedup keys kept in memory.
/// Covers all JetStream redelivery windows while bounding heap growth (H-4).
const FILL_CACHE_CAPACITY: usize = 50_000;

/// A single fill event received from the broker.
#[derive(Debug, Clone)]
pub struct FillEvent {
    /// Matches `OrderIntent.idempotency_key` and the `orders` table.
    pub idempotency_key: Uuid,
    pub broker_order_id: String,
    pub filled_qty: Decimal,
    pub fill_price: Price,
    pub commission: Decimal,
    pub filled_at: DateTime<Utc>,
}

/// Outcome of applying a fill to the processor.
#[derive(Debug, PartialEq, Eq)]
pub enum FillResult {
    /// Fill was new and applied successfully.
    Applied,
    /// Fill was already seen; skipped as a no-op.
    Duplicate,
}

/// In-memory dedup key for a fill.  In production the DB `fills` table's
/// UNIQUE constraint on `(idempotency_key, broker_order_id, fill_price, filled_qty)`
/// provides the durable idempotency guarantee; this struct provides the
/// in-memory fast path used in unit tests and by the `ExecutionEngine`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FillDedupKey {
    pub idempotency_key: Uuid,
    pub broker_order_id: String,
    /// Serialised to string to make it Eq/Hash without float comparison.
    pub fill_price_str: String,
    pub filled_qty_str: String,
}

impl FillDedupKey {
    pub fn from_event(ev: &FillEvent) -> Self {
        Self {
            idempotency_key: ev.idempotency_key,
            broker_order_id: ev.broker_order_id.clone(),
            fill_price_str: ev.fill_price.inner().to_string(),
            filled_qty_str: ev.filled_qty.to_string(),
        }
    }
}

/// Stateful fill processor that tracks seen fills in memory.
///
/// In production this is backed by the `fills` Postgres table, but the
/// idempotency logic is encapsulated here so it can be unit-tested without I/O.
///
/// Uses a bounded FIFO cache so memory usage is capped regardless of session
/// length or fill volume (H-4).
#[derive(Debug)]
pub struct FillProcessor {
    seen: std::collections::HashSet<FillDedupKey>,
    order: VecDeque<FillDedupKey>,
    capacity: usize,
}

impl Default for FillProcessor {
    fn default() -> Self {
        Self::with_capacity(FILL_CACHE_CAPACITY)
    }
}

impl FillProcessor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            seen: std::collections::HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Apply `fill`.  Returns `Duplicate` if already seen, `Applied` otherwise.
    pub fn apply(&mut self, fill: &FillEvent) -> FillResult {
        let key = FillDedupKey::from_event(fill);
        if self.seen.contains(&key) {
            FillResult::Duplicate
        } else {
            if self.order.len() >= self.capacity {
                if let Some(oldest) = self.order.pop_front() {
                    self.seen.remove(&oldest);
                }
            }
            self.order.push_back(key.clone());
            self.seen.insert(key);
            FillResult::Applied
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn fill(broker_id: &str, qty: &str, price: &str) -> FillEvent {
        FillEvent {
            idempotency_key: Uuid::new_v4(),
            broker_order_id: broker_id.to_owned(),
            filled_qty: Decimal::from_str(qty).unwrap(),
            fill_price: Price::from_str(price).unwrap(),
            commission: Decimal::ZERO,
            filled_at: Utc::now(),
        }
    }

    #[test]
    fn first_fill_is_applied() {
        let mut fp = FillProcessor::new();
        let f = fill("order-1", "1.0", "50000");
        assert_eq!(fp.apply(&f), FillResult::Applied);
    }

    #[test]
    fn identical_fill_is_duplicate() {
        let mut fp = FillProcessor::new();
        let f = fill("order-1", "1.0", "50000");
        fp.apply(&f);
        assert_eq!(fp.apply(&f), FillResult::Duplicate);
    }

    #[test]
    fn different_qty_is_new_fill() {
        let mut fp = FillProcessor::new();
        let mut f1 = fill("order-1", "0.5", "50000");
        let key = f1.idempotency_key;
        fp.apply(&f1);
        f1.idempotency_key = key;
        f1.filled_qty = Decimal::from_str("1.0").unwrap(); // different qty
        assert_eq!(fp.apply(&f1), FillResult::Applied);
    }
}
