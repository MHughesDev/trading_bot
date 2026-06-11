//! Demand registry: tracks (lane, instrument) consumer counts and manages pipeline lifecycle.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use domain::lanes::Lane;
use tokio::sync::oneshot;
use tracing::debug;

use crate::lane_key::LaneKey;
use crate::lifecycle::{PipelineFactory, PipelineHandle};

struct DemandEntry {
    count: u32,
    handle: Option<PipelineHandle>,
}

struct TeardownState {
    cancel_tx: oneshot::Sender<()>,
}

struct KeyedEntry {
    count: u32,
    handle: Option<PipelineHandle>,
    teardown_cancel: Option<TeardownState>,
}

/// Aggregates `(lane, instrument)` demand across consumers and starts/stops pipelines.
///
/// - On the first `add()` call (0 → 1), the factory spawns a pipeline.
/// - On the last `remove()` call (1 → 0), the pipeline is stopped.
/// - Intermediate adds/removes adjust the count without starting/stopping.
pub struct DemandRegistry {
    factory: Arc<dyn PipelineFactory>,
    entries: DashMap<(Arc<str>, Arc<str>), DemandEntry>,
    keyed: Arc<DashMap<LaneKey, KeyedEntry>>,
}

impl DemandRegistry {
    pub fn new(factory: Arc<dyn PipelineFactory>) -> Self {
        Self {
            factory,
            entries: DashMap::new(),
            keyed: Arc::new(DashMap::new()),
        }
    }

    // ── Legacy API ────────────────────────────────────────────────────────────

    /// Declare demand for `(lane, instrument)`.  Starts the pipeline on 0 → 1.
    pub fn add(&self, lane: &Lane, instrument: &str) {
        let key: (Arc<str>, Arc<str>) = (
            Arc::from(lane.as_str()),
            Arc::from(instrument),
        );
        let needs_start = {
            let mut entry = self.entries.entry(key.clone()).or_insert(DemandEntry {
                count: 0,
                handle: None,
            });
            entry.count += 1;
            entry.count == 1
        };
        if needs_start {
            debug!(lane = lane.as_str(), instrument, "pipeline starting");
            let handle = self.factory.start(lane, instrument);
            if let Some(mut entry) = self.entries.get_mut(&key) {
                entry.handle = Some(handle);
            }
        }
    }

    /// Remove demand for `(lane, instrument)`.  Stops the pipeline on 1 → 0.
    pub fn remove(&self, lane: &Lane, instrument: &str) {
        let key: (Arc<str>, Arc<str>) = (
            Arc::from(lane.as_str()),
            Arc::from(instrument),
        );
        let stop_handle = {
            if let Some(mut entry) = self.entries.get_mut(&key) {
                if entry.count > 0 {
                    entry.count -= 1;
                }
                if entry.count == 0 {
                    let h = entry.handle.take();
                    drop(entry);
                    self.entries.remove(&key);
                    h
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(handle) = stop_handle {
            debug!(lane = lane.as_str(), instrument, "pipeline stopping");
            handle.stop();
        }
    }

    /// Returns the current demand count for `(lane, instrument)`.
    pub fn count(&self, lane: &Lane, instrument: &str) -> u32 {
        let key: (Arc<str>, Arc<str>) = (
            Arc::from(lane.as_str()),
            Arc::from(instrument),
        );
        self.entries.get(&key).map(|e| e.count).unwrap_or(0)
    }

    // ── Keyed API (LaneKey + 120-second warm period) ──────────────────────────

    /// Declare demand for a `LaneKey`.  Starts the pipeline on 0 → 1.
    /// If a teardown is pending (warm period), cancels it instead of restarting.
    pub fn acquire(&self, key: LaneKey) {
        let mut entry = self.keyed.entry(key.clone()).or_insert(KeyedEntry {
            count: 0,
            handle: None,
            teardown_cancel: None,
        });

        if let Some(state) = entry.teardown_cancel.take() {
            let _ = state.cancel_tx.send(());
            debug!(lane = %key, "teardown cancelled; reusing running pipeline");
            entry.count += 1;
            return;
        }

        entry.count += 1;
        if entry.count == 1 {
            debug!(lane = %key, "keyed pipeline starting");
            let lane = key
                .data_type
                .as_key()
                .parse::<Lane>()
                .unwrap_or(Lane::MarketBars1m);
            let handle = self.factory.start(&lane, &key.instrument_id);
            entry.handle = Some(handle);
        }
    }

    /// Release demand for a `LaneKey`.  When count reaches zero, starts the
    /// 120-second warm period before tearing down the pipeline.
    pub fn release(&self, key: LaneKey) {
        let (should_schedule, handle_opt) = {
            if let Some(mut entry) = self.keyed.get_mut(&key) {
                if entry.count > 0 {
                    entry.count -= 1;
                }
                if entry.count == 0 && entry.teardown_cancel.is_none() {
                    let handle = entry.handle.take();
                    (true, handle)
                } else {
                    (false, None)
                }
            } else {
                (false, None)
            }
        };

        if should_schedule {
            let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
            if let Some(mut entry) = self.keyed.get_mut(&key) {
                entry.teardown_cancel = Some(TeardownState { cancel_tx });
            }

            let keyed = Arc::clone(&self.keyed);
            let key_clone = key.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(120)) => {
                        if let Some(mut e) = keyed.get_mut(&key_clone) {
                            e.teardown_cancel = None;
                            if e.count == 0 {
                                drop(e);
                                keyed.remove(&key_clone);
                            }
                        }
                        debug!(lane = %key_clone, "keyed pipeline torn down after warm period");
                        if let Some(h) = handle_opt {
                            h.stop();
                        }
                    }
                    _ = cancel_rx => {
                        debug!(lane = %key_clone, "warm-period teardown cancelled");
                    }
                }
            });
        }
    }

    /// Current acquire count for a `LaneKey` (for testing).
    pub fn acquire_count(&self, key: &LaneKey) -> u32 {
        self.keyed.get(key).map(|e| e.count).unwrap_or(0)
    }
}
