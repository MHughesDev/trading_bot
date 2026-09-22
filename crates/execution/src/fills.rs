//! Idempotent fill processing.
//!
//! Fill processing is keyed by `(idempotency_key, broker_order_id, filled_qty, fill_price)`.
//! A replay of the same fill from JetStream is a no-op.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

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
///
/// "Cannot fail" is a contract, not a guarantee — an observer can still panic.
/// [`publish`] therefore isolates each delivery and *counts* the ones that did
/// not land ([`FillStreamEvidence::observer_failures`]), because the failure
/// mode this seam exists to prevent is a fill that is taken and quietly
/// recorded nowhere.
pub trait FillObserver: Send + Sync {
    fn on_fill(&self, fill: &FillEvent);
}

static OBSERVERS: std::sync::RwLock<Vec<std::sync::Arc<dyn FillObserver>>> =
    std::sync::RwLock::new(Vec::new());

/// Every fill handed to [`publish`].
static FILLS_PUBLISHED: AtomicU64 = AtomicU64::new(0);
/// Fills published while **no** observer was registered.  These are in the
/// `fills` table and in no forward record, and no later registration recovers
/// them: the fill stream has no replay.
static FILLS_UNOBSERVED: AtomicU64 = AtomicU64::new(0);
/// Deliveries that unwound inside an observer.  The fill reached the registry
/// and not that observer.
static OBSERVER_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Subscribe to the fill stream.  Idempotent per observer instance is *not*
/// enforced — register once, at startup.
///
/// A poisoned registry is recovered rather than skipped.  Dropping a
/// registration because some unrelated thread panicked would leave a process
/// that believes it is recording and is not, which is the one state this
/// module is built to make impossible.
pub fn register_observer(observer: std::sync::Arc<dyn FillObserver>) {
    let mut guard = OBSERVERS.write().unwrap_or_else(|poisoned| {
        tracing::error!("fill observer registry was poisoned — recovering it to register anyway");
        poisoned.into_inner()
    });
    guard.push(observer);
}

/// Number of registered observers, for diagnostics and tests.
#[must_use]
pub fn observer_count() -> usize {
    OBSERVERS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
}

/// What the fill stream can honestly say about itself.
///
/// Gate 15 reads forward observations.  Zero of them has two causes that look
/// identical from the outside and mean opposite things: *nothing traded*, and
/// *things traded and nobody was listening*.  These counters are what separates
/// them, so an empty forward record can be interrogated instead of assumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillStreamEvidence {
    /// Observers registered right now.
    pub observers: usize,
    /// Fills handed to [`publish`] since process start.
    pub fills_published: u64,
    /// Of those, the ones that reached no observer at all.
    pub fills_unobserved: u64,
    /// Deliveries lost to a panicking observer.
    pub observer_failures: u64,
}

impl FillStreamEvidence {
    /// True when at least one observer is attached *now*.  Says nothing about
    /// the past — see [`Self::evidence_gap`].
    #[must_use]
    pub fn is_recording(&self) -> bool {
        self.observers > 0
    }

    /// Fills that happened and were not recorded by anyone.  Never decreases;
    /// attaching an observer later does not repair a fill already taken.
    #[must_use]
    pub fn evidence_gap(&self) -> u64 {
        self.fills_unobserved + self.observer_failures
    }

    /// The three distinguishable states of the forward-evidence clock.
    #[must_use]
    pub fn verdict(&self) -> FillStreamVerdict {
        let gap = self.evidence_gap();
        if gap > 0 {
            FillStreamVerdict::EvidenceLost { fills: gap }
        } else if self.is_recording() {
            FillStreamVerdict::Recording {
                fills: self.fills_published,
            }
        } else {
            FillStreamVerdict::NotRecording
        }
    }
}

/// Why the forward record holds the number of rows it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillStreamVerdict {
    /// Nobody is listening and nothing has been lost yet.  An empty forward
    /// record in this state measures nothing at all — it is not a zero.
    NotRecording,
    /// Fills were taken that reached no observer.  The forward record is
    /// missing rows it can never be given, and any statistic computed over it
    /// is computed over a biased sample.
    EvidenceLost { fills: u64 },
    /// Every published fill reached at least one observer.  An empty forward
    /// record in this state genuinely means no fills happened.
    Recording { fills: u64 },
}

/// Read the fill stream's self-report.
#[must_use]
pub fn evidence() -> FillStreamEvidence {
    FillStreamEvidence {
        observers: observer_count(),
        fills_published: FILLS_PUBLISHED.load(Ordering::Relaxed),
        fills_unobserved: FILLS_UNOBSERVED.load(Ordering::Relaxed),
        observer_failures: OBSERVER_FAILURES.load(Ordering::Relaxed),
    }
}

/// The reason a process must not be trusted to produce forward evidence.
#[derive(Debug, thiserror::Error)]
pub enum FillStreamGap {
    #[error(
        "no fill observer is registered: fills would be taken and no forward observation recorded. \
         Attach one (execution::fills::register_observer) before the first order is submitted"
    )]
    NoObserver,
    #[error(
        "{fills} fill(s) reached no observer; the forward record is missing rows that cannot be \
         recovered — the fill stream has no replay"
    )]
    EvidenceLost { fills: u64 },
}

/// Startup guard: fail loudly if this process would trade without recording.
///
/// Called by the binary *before* the first order can be submitted.  It is the
/// difference between discovering the observer was never attached now and
/// discovering it after a paper credential and a month of calendar time have
/// been spent producing an empty forward record.
///
/// # Errors
/// [`FillStreamGap::NoObserver`] when nothing is listening;
/// [`FillStreamGap::EvidenceLost`] when fills have already slipped past.
pub fn require_recording() -> Result<FillStreamEvidence, FillStreamGap> {
    let ev = evidence();
    match ev.verdict() {
        FillStreamVerdict::NotRecording => Err(FillStreamGap::NoObserver),
        FillStreamVerdict::EvidenceLost { fills } => Err(FillStreamGap::EvidenceLost { fills }),
        FillStreamVerdict::Recording { .. } => Ok(ev),
    }
}

/// Publish a fill to every observer.
///
/// Called by [`crate::ExecutionEngine`] only when the fill was both new to the
/// in-memory processor (`FillResult::Applied`) **and** newly inserted into the
/// `fills` table.  Those two together are the definition of "this fill happened
/// and we had not seen it before"; either alone is not.
///
/// Three things can go wrong here and none of them are allowed to be quiet:
///
/// * **No observer.** The fill is counted in [`FillStreamEvidence::fills_unobserved`]
///   and logged at `error`.  It is not a no-op, because a no-op is
///   indistinguishable from a process that never traded.
/// * **A poisoned registry.** Recovered and delivered anyway.  A fill dropped
///   because an unrelated thread panicked is the same lost evidence by a
///   different route.
/// * **A panicking observer.** Isolated, counted, and the remaining observers
///   still receive the fill.  (Under `profile.release`'s `panic = "abort"` the
///   process dies instead — also loud, also not silent.)
pub fn publish(fill: &FillEvent) {
    FILLS_PUBLISHED.fetch_add(1, Ordering::Relaxed);

    let observers: Vec<std::sync::Arc<dyn FillObserver>> = {
        let guard = OBSERVERS.read().unwrap_or_else(|poisoned| {
            tracing::error!(
                idempotency_key = %fill.idempotency_key,
                "fill observer registry was poisoned — recovering it to publish anyway"
            );
            poisoned.into_inner()
        });
        guard.iter().map(std::sync::Arc::clone).collect()
    };

    if observers.is_empty() {
        FILLS_UNOBSERVED.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            idempotency_key = %fill.idempotency_key,
            broker_order_id = %fill.broker_order_id,
            filled_qty = %fill.filled_qty,
            fills_unobserved = FILLS_UNOBSERVED.load(Ordering::Relaxed),
            "a fill was taken with no fill observer registered — it is in the journal and in no \
             forward record. This is an evidence gap, not an absence of trading"
        );
        return;
    }

    for observer in &observers {
        let delivered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observer.on_fill(fill);
        }));
        if delivered.is_err() {
            OBSERVER_FAILURES.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                idempotency_key = %fill.idempotency_key,
                broker_order_id = %fill.broker_order_id,
                "a fill observer panicked — that observer did not record this fill"
            );
        }
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
