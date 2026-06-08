//! Global kill switch — synchronous in-memory flag with Postgres persistence.
//!
//! The hot-path check (`is_active`) never blocks: it reads an `AtomicBool`.
//! Persistence to Postgres is the caller's responsibility and can happen
//! asynchronously after tripping.

use std::sync::atomic::{AtomicBool, Ordering};

/// Global trading gate.  `true` = halted; `false` = trading allowed.
pub struct KillSwitch {
    active: AtomicBool,
}

impl KillSwitch {
    pub fn new(initially_active: bool) -> Self {
        Self {
            active: AtomicBool::new(initially_active),
        }
    }

    /// Returns `true` if the kill switch is currently tripped (trading halted).
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Trip the kill switch — blocks all new orders.
    pub fn trip(&self) {
        self.active.store(true, Ordering::Release);
        tracing::warn!("kill switch tripped — all new order flow halted");
    }

    /// Reset the kill switch — resumes order flow.  Use with care.
    pub fn reset(&self) {
        self.active.store(false, Ordering::Release);
        tracing::info!("kill switch reset — order flow resumed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_switch_defaults_inactive() {
        let ks = KillSwitch::new(false);
        assert!(!ks.is_active());
    }

    #[test]
    fn new_switch_can_start_active() {
        let ks = KillSwitch::new(true);
        assert!(ks.is_active());
    }

    #[test]
    fn trip_activates() {
        let ks = KillSwitch::new(false);
        ks.trip();
        assert!(ks.is_active());
    }

    #[test]
    fn reset_deactivates() {
        let ks = KillSwitch::new(true);
        ks.reset();
        assert!(!ks.is_active());
    }
}
