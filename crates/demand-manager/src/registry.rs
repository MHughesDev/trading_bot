//! Demand registry: tracks (lane, instrument) consumer counts and manages pipeline lifecycle.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use domain::lanes::Lane;
use tracing::debug;

use crate::lifecycle::{PipelineFactory, PipelineHandle};

struct DemandEntry {
    count: u32,
    handle: Option<PipelineHandle>,
}

/// Aggregates `(lane, instrument)` demand across consumers and starts/stops pipelines.
///
/// - On the first `add()` call (0 → 1), the factory spawns a pipeline.
/// - On the last `remove()` call (1 → 0), the pipeline is stopped.
/// - Intermediate adds/removes adjust the count without starting/stopping.
pub struct DemandRegistry {
    factory: Arc<dyn PipelineFactory>,
    entries: Mutex<HashMap<(String, String), DemandEntry>>,
}

impl DemandRegistry {
    pub fn new(factory: Arc<dyn PipelineFactory>) -> Self {
        Self {
            factory,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Declare demand for `(lane, instrument)`.  Starts the pipeline on 0 → 1.
    pub fn add(&self, lane: &Lane, instrument: &str) {
        let key = (lane.as_str().to_owned(), instrument.to_owned());
        let needs_start = {
            let mut map = self.entries.lock().unwrap();
            let entry = map.entry(key.clone()).or_insert(DemandEntry {
                count: 0,
                handle: None,
            });
            entry.count += 1;
            entry.count == 1
        };
        if needs_start {
            debug!(lane = lane.as_str(), instrument, "pipeline starting");
            let handle = self.factory.start(lane, instrument);
            let mut map = self.entries.lock().unwrap();
            if let Some(entry) = map.get_mut(&key) {
                entry.handle = Some(handle);
            }
        }
    }

    /// Remove demand for `(lane, instrument)`.  Stops the pipeline on 1 → 0.
    pub fn remove(&self, lane: &Lane, instrument: &str) {
        let key = (lane.as_str().to_owned(), instrument.to_owned());
        let stop_handle = {
            let mut map = self.entries.lock().unwrap();
            if let Some(entry) = map.get_mut(&key) {
                if entry.count > 0 {
                    entry.count -= 1;
                }
                if entry.count == 0 {
                    let h = entry.handle.take();
                    map.remove(&key);
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
        let key = (lane.as_str().to_owned(), instrument.to_owned());
        let map = self.entries.lock().unwrap();
        map.get(&key).map(|e| e.count).unwrap_or(0)
    }
}
