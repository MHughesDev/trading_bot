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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillResult {
    /// Fill was new and applied successfully.
    Applied,
    /// Fill was already seen; skipped as a no-op.
    Duplicate,
}

// ── The fill stream ──────────────────────────────────────────────────────────

/// Something that wants to hear about every fill the platform takes.
///
/// This is the seam reconciliation, the forward-observation collector and
/// Gate 15's evidence path attach to.  It fires **once per genuinely new
/// fill** — a redelivery, a resubmit, or a restart replaying the same broker
/// fill does not fire it again, because an observer that counts a fill twice
/// reports a forward observation that never happened.
///
/// Added for Set T task T-0.1: `crates/reconciliation` existed and nothing
/// called it, so the platform's most expensive unanswered question — does the
/// simulator tell the truth? — had no input.
///
/// Observers are notified synchronously, must not block (do the work on your
/// own task), and cannot fail: an observer that could fail would need a policy
/// for what to do when it does, and every such policy either drops evidence or
/// blocks the execution path.
pub trait FillObserver: Send + Sync {
    fn on_fill(&self, fill: &FillEvent);
}

static OBSERVERS: std::sync::RwLock<Vec<std::sync::Arc<dyn FillObserver>>> =
    std::sync::RwLock::new(Vec::new());

/// Subscribe to the fill stream.  Idempotent per observer instance is *not*
/// enforced — register once, at startup.
pub fn register_observer(observer: std::sync::Arc<dyn FillObserver>) {
    match OBSERVERS.write() {
        Ok(mut w) => w.push(observer),
        Err(_) => tracing::error!(
            "fill observer registry poisoned — this observer will not receive fills"
        ),
    }
}

/// Number of registered observers, for diagnostics and tests.
#[must_use]
pub fn observer_count() -> usize {
    OBSERVERS.read().map(|r| r.len()).unwrap_or(0)
}

/// Publish a fill to every observer.
///
/// Called by [`crate::ExecutionEngine`] only when the fill was both new to the
/// in-memory processor (`FillResult::Applied`) **and** newly inserted into the
/// `fills` table.  Those two together are the definition of "this fill happened
/// and we had not seen it before"; either alone is not.
pub fn publish(fill: &FillEvent) {
    let observers = match OBSERVERS.read() {
        Ok(r) => r.clone(),
        Err(_) => {
            tracing::error!(
                idempotency_key = %fill.idempotency_key,
                "fill observer registry poisoned — a fill was not published"
            );
            return;
        }
    };
    for observer in &observers {
        observer.on_fill(fill);
    }
}

/// In-memory dedup key for a fill.
///
/// This is a **latency** cache and nothing more.  An earlier version of this
/// comment claimed the `fills` UNIQUE constraint provided "the durable
/// idempotency guarantee"; at the time neither the constraint nor any
/// `INSERT INTO fills` existed, so the only thing standing between a
/// redelivered fill and a double-applied position was the bounded `VecDeque`
/// below — which a restart wipes, and a restart is exactly the reconnect SC-7
/// is about.  The durable guarantee is now real and lives in
/// [`crate::journal`]: `fills_idempotent` (migration 0121) plus
/// `ON CONFLICT DO NOTHING`.  Both run on every fill; this one first because
/// it is cheap, the database second because it is true.
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
/// Not a durability mechanism — see [`FillDedupKey`].  The authoritative
/// answer to "have we already applied this fill?" is the `fills` table's
/// unique constraint, checked in [`crate::journal::OrderJournal::record_fill`].
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
