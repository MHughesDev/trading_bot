//! Ref-counted start/stop of collector instances per (venue_id, lane, instrument).

use std::sync::Arc;

use collectors::Collector;
use event_bus::{Publisher, QuarantinePublisher};
use tracing::{info, warn};

use super::registry::CollectorRegistry;

/// Manages collector lifecycle using demand ref-counting.
///
/// A collector is spawned on the first [`demand`] call and aborted when
/// [`release`] drops the count to zero.
pub struct LifecycleManager {
    registry: CollectorRegistry,
    publisher: Arc<Publisher>,
    quarantine: Arc<QuarantinePublisher>,
}

impl LifecycleManager {
    /// Create a new `LifecycleManager`.
    pub fn new(publisher: Arc<Publisher>, quarantine: Arc<QuarantinePublisher>) -> Self {
        Self {
            registry: CollectorRegistry::new(),
            publisher,
            quarantine,
        }
    }

    /// Declare demand for a (venue_id, lane, instrument_id) stream.
    ///
    /// Increments the ref-count.  If this is the first subscriber the collector
    /// is spawned as a background Tokio task.
    pub async fn demand(
        &self,
        venue_id: String,
        lane: String,
        instrument_id: String,
        collector: Arc<dyn Collector>,
    ) {
        let key = (venue_id.clone(), lane.clone(), instrument_id.clone());
        let is_first = self.registry.incr(key.clone()).await;

        if is_first {
            info!(
                venue_id = %venue_id,
                lane = %lane,
                instrument_id = %instrument_id,
                "starting collector"
            );

            let publisher = Arc::clone(&self.publisher);
            let quarantine = Arc::clone(&self.quarantine);

            let handle = tokio::spawn(async move {
                if let Err(e) = collector.run(publisher, quarantine).await {
                    warn!(
                        venue_id = %venue_id,
                        lane = %lane,
                        instrument_id = %instrument_id,
                        error = %e,
                        "collector exited with error"
                    );
                }
            });

            self.registry.insert_handle(key, handle).await;
        }
    }

    /// Release demand for a (venue_id, lane, instrument_id) stream.
    ///
    /// Decrements the ref-count.  When the count reaches zero the collector task
    /// is aborted.
    pub async fn release(&self, venue_id: &str, lane: &str, instrument_id: &str) {
        let key = (
            venue_id.to_owned(),
            lane.to_owned(),
            instrument_id.to_owned(),
        );
        let should_stop = self.registry.decr(&key).await;

        if should_stop {
            info!(
                venue_id = %venue_id,
                lane = %lane,
                instrument_id = %instrument_id,
                "stopping collector (no more subscribers)"
            );
            self.registry.remove_handle(&key).await;
        }
    }
}
