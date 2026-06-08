//! Strategy clock abstraction.
//!
//! Live instances use `WallClock`; replay instances use `ReplayClock`, which
//! advances under the caller's control so `world.now()` tracks `available_time`
//! of the dispatched event rather than the OS clock.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

/// Abstraction over the clock that a strategy instance reads.
///
/// Strategies must call `world.now()` rather than any OS time function; the
/// runtime wires the right implementation so live and replay behave identically.
pub trait StrategyClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Real wall-clock time.  Used by live instances.
pub struct WallClock;

impl StrategyClock for WallClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Simulated clock advanced by the replay loop.
///
/// The runtime advances this to each event's `available_time` before dispatching
/// the event to the strategy, so `world.now()` returns the event time.
pub struct ReplayClock {
    current: Mutex<DateTime<Utc>>,
}

impl ReplayClock {
    pub fn new(start: DateTime<Utc>) -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(start),
        })
    }

    /// Advance the clock to `t`.  Called by the replay loop before each event.
    pub fn advance(&self, t: DateTime<Utc>) {
        *self.current.lock().expect("ReplayClock mutex") = t;
    }
}

impl StrategyClock for ReplayClock {
    fn now(&self) -> DateTime<Utc> {
        *self.current.lock().expect("ReplayClock mutex")
    }
}
