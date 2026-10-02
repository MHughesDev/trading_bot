//! Time and identity for a venue (ADR-0044, Set U U-2.1).
//!
//! The paper engine read the wall clock in a dozen places and minted order ids
//! from `Uuid::new_v4`. Both are right for paper and wrong for a backtest: a
//! simulated venue must stamp a fill with the *simulated* instant — otherwise
//! the daily-loss tally, which buckets realised P&L by calendar day, would put
//! every backtest trade on the day the backtest ran — and must mint the same ids
//! on every run, or two identical backtests are not byte-identical.
//!
//! So the venue core asks a [`VenueClock`] for the time and an [`OrderIds`] for
//! ids, and the two venues differ only in which they are given: [`WallClock`] and
//! [`RandomIds`] for paper, [`SimClock`] and [`SequentialIds`] for a backtest.

use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};

/// Where a venue gets the time.
pub trait VenueClock: Send + Sync + Debug {
    fn now(&self) -> DateTime<Utc>;
}

/// The wall clock — paper and live.
#[derive(Clone, Copy, Debug, Default)]
pub struct WallClock;

impl VenueClock for WallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock the backtest driver sets — never reads the wall.
#[derive(Debug)]
pub struct SimClock {
    now: Mutex<DateTime<Utc>>,
}

impl SimClock {
    #[must_use]
    pub fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Move the clock. Never backwards: a venue that saw time reverse would
    /// fill an order before it was placed.
    pub fn set(&self, t: DateTime<Utc>) {
        let mut now = self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if t > *now {
            *now = t;
        }
    }
}

impl VenueClock for SimClock {
    fn now(&self) -> DateTime<Utc> {
        *self
            .now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Where a venue gets order ids.
pub trait OrderIds: Send + Sync + Debug {
    fn next_id(&self) -> String;
}

/// Random v4 UUIDs — paper and live, where ids must never collide across
/// restarts.
#[derive(Clone, Copy, Debug, Default)]
pub struct RandomIds;

impl OrderIds for RandomIds {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// `sim-000000000001`, `sim-000000000002`, … — a backtest, where the same run
/// must produce the same ids.
#[derive(Debug, Default)]
pub struct SequentialIds {
    n: AtomicU64,
}

impl OrderIds for SequentialIds {
    fn next_id(&self) -> String {
        format!("sim-{:012}", self.n.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sim_clock_never_runs_backwards() {
        let t0 = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let c = SimClock::new(t0);
        c.set(t0 + chrono::Duration::hours(1));
        c.set(t0);
        assert_eq!(c.now(), t0 + chrono::Duration::hours(1));
    }

    #[test]
    fn sequential_ids_are_the_same_every_run() {
        let a = SequentialIds::default();
        let b = SequentialIds::default();
        let xs: Vec<String> = (0..3).map(|_| a.next_id()).collect();
        let ys: Vec<String> = (0..3).map(|_| b.next_id()).collect();
        assert_eq!(xs, ys);
        assert_eq!(xs[0], "sim-000000000001");
    }
}
