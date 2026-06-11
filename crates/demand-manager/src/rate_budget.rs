//! Server-wide rate-limit admission control (P2-T02).
//!
//! Each venue declares a free-tier budget (requests/min, max concurrent subscriptions).
//! `RateBudget::try_admit` admits or denies new collector subscriptions against
//! per-venue budgets.  The budget is a single server-wide resource shared across all users.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use domain::SupportedVenue;
use thiserror::Error;

/// Free-tier capacity for one venue.
#[derive(Debug)]
pub struct VenueBudget {
    /// Maximum concurrent active subscriptions for this venue.
    pub max_concurrent: u32,
    /// Currently active subscription count (atomic for lock-free increment/decrement).
    active: AtomicU32,
}

impl VenueBudget {
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            max_concurrent,
            active: AtomicU32::new(0),
        }
    }
}

/// Returned when the admission request exceeds the venue's budget.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("budget exceeded for {venue}: active={active}, max={max}")]
pub struct BudgetExceeded {
    pub venue: String,
    pub active: u32,
    pub max: u32,
}

/// Server-wide rate-limit admission control.
///
/// Before starting a new collector lane, call `try_admit`.  Call `release` when
/// the lane stops.
pub struct RateBudget {
    budgets: Mutex<HashMap<SupportedVenue, VenueBudget>>,
}

impl RateBudget {
    /// Construct with per-venue budgets.
    pub fn new(budgets: impl IntoIterator<Item = (SupportedVenue, VenueBudget)>) -> Self {
        Self {
            budgets: Mutex::new(budgets.into_iter().collect()),
        }
    }

    /// Build a `RateBudget` with sensible free-tier defaults for all venues.
    pub fn with_defaults() -> Self {
        let defaults = [
            (SupportedVenue::Kraken, VenueBudget::new(50)),
            (SupportedVenue::Coinbase, VenueBudget::new(50)),
            (SupportedVenue::Alpaca, VenueBudget::new(30)),
            (SupportedVenue::Oanda, VenueBudget::new(20)),
            (SupportedVenue::Kalshi, VenueBudget::new(20)),
            (SupportedVenue::Tradier, VenueBudget::new(20)),
            (SupportedVenue::ZeroX, VenueBudget::new(10)),
            (SupportedVenue::Tradovate, VenueBudget::new(10)),
        ];
        Self::new(defaults)
    }

    /// Attempt to admit a new subscription for `venue` (cost = 1 slot).
    ///
    /// Returns `Ok(())` on success (slot consumed) or `Err(BudgetExceeded)` if the
    /// venue's concurrent limit is already reached.
    pub fn try_admit(&self, venue: SupportedVenue) -> Result<(), BudgetExceeded> {
        let mut budgets = self.budgets.lock().unwrap();
        let budget = budgets.entry(venue).or_insert_with(|| VenueBudget::new(10));

        let prev = budget.active.fetch_add(1, Ordering::Relaxed);
        if prev >= budget.max_concurrent {
            budget.active.fetch_sub(1, Ordering::Relaxed);
            return Err(BudgetExceeded {
                venue: venue.as_str().to_owned(),
                active: prev,
                max: budget.max_concurrent,
            });
        }
        Ok(())
    }

    /// Release one slot for `venue` (call when a lane stops).
    pub fn release(&self, venue: SupportedVenue) {
        let budgets = self.budgets.lock().unwrap();
        if let Some(budget) = budgets.get(&venue) {
            // saturating_sub via compare to avoid wrapping below zero
            budget
                .active
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                    Some(v.saturating_sub(1))
                })
                .ok();
        }
    }

    /// Current active slot count for `venue` (for testing).
    pub fn active(&self, venue: SupportedVenue) -> u32 {
        let budgets = self.budgets.lock().unwrap();
        budgets
            .get(&venue)
            .map(|b| b.active.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}
