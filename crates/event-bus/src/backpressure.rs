//! Semaphore-based in-flight backpressure.

use std::sync::Arc;
use tokio::sync::{Semaphore, SemaphorePermit};

/// Limits how many messages can be in flight simultaneously.
pub struct Backpressure {
    sem: Arc<Semaphore>,
}

impl Backpressure {
    /// Create a new `Backpressure` with the given maximum in-flight count.
    pub fn new(max_in_flight: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(max_in_flight)),
        }
    }

    /// Acquire a permit, waiting if the limit is already reached.
    pub async fn acquire(&self) -> SemaphorePermit<'_> {
        self.sem
            .acquire()
            .await
            .expect("semaphore should never be closed")
    }
}
