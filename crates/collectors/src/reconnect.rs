//! Exponential backoff reconnect policy.
//!
//! Each collector holds a [`ReconnectPolicy`] and calls [`ReconnectPolicy::wait`]
//! between connection attempts.  The delay doubles on each attempt, capping at
//! `max_ms`.  After a successful connection the caller should call
//! [`ReconnectPolicy::reset`].

use tokio::time::{sleep, Duration};

/// Exponential backoff configuration.
pub struct ReconnectPolicy {
    initial_ms: u64,
    max_ms: u64,
    current_ms: u64,
    attempt: u32,
}

impl ReconnectPolicy {
    /// Create a new policy with the given initial and maximum delay in milliseconds.
    pub fn new(initial_ms: u64, max_ms: u64) -> Self {
        Self {
            initial_ms,
            max_ms,
            current_ms: initial_ms,
            attempt: 0,
        }
    }

    /// Sleep for the current backoff duration, then double it up to `max_ms`.
    pub async fn wait(&mut self) {
        sleep(Duration::from_millis(self.current_ms)).await;
        self.attempt = self.attempt.saturating_add(1);
        self.current_ms = (self.current_ms * 2).min(self.max_ms);
    }

    /// Reset the policy after a successful connection.
    pub fn reset(&mut self) {
        self.current_ms = self.initial_ms;
        self.attempt = 0;
    }

    /// Current backoff delay in milliseconds.
    pub fn current_ms(&self) -> u64 {
        self.current_ms
    }

    /// Number of reconnect attempts since the last reset.
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self::new(500, 30_000)
    }
}
