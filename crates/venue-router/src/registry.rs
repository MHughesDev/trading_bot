//! Ref-counted registry of running collectors per (venue_id, lane, instrument_id).
//!
//! The registry tracks how many consumers have demanded each stream.  A collector
//! is only spawned when the first subscriber arrives and is stopped when the last
//! one leaves.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Composite key: (venue_id, lane, instrument_id).
type Key = (String, String, String);

/// Tracks subscriber counts and running task handles.
pub struct CollectorRegistry {
    counts: Arc<Mutex<HashMap<Key, u32>>>,
    handles: Arc<Mutex<HashMap<Key, JoinHandle<()>>>>,
}

impl CollectorRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            counts: Arc::new(Mutex::new(HashMap::new())),
            handles: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Increment the demand ref-count for `key`.
    ///
    /// Returns `true` when this is the **first** subscriber (count transitions
    /// from 0 → 1) — the caller should spawn the collector.
    pub async fn incr(&self, key: Key) -> bool {
        let mut counts = self.counts.lock().await;
        let entry = counts.entry(key).or_insert(0);
        *entry += 1;
        *entry == 1
    }

    /// Decrement the demand ref-count for `key`.
    ///
    /// Returns `true` when the count reaches zero — the caller should stop the
    /// collector.
    pub async fn decr(&self, key: &Key) -> bool {
        let mut counts = self.counts.lock().await;
        if let Some(count) = counts.get_mut(key) {
            if *count > 0 {
                *count -= 1;
            }
            if *count == 0 {
                counts.remove(key);
                return true;
            }
        }
        false
    }

    /// Store a running task handle for `key`.
    pub async fn insert_handle(&self, key: Key, handle: JoinHandle<()>) {
        let mut handles = self.handles.lock().await;
        handles.insert(key, handle);
    }

    /// Abort and remove the task handle for `key`.
    pub async fn remove_handle(&self, key: &Key) {
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.remove(key) {
            handle.abort();
        }
    }

    /// Return the current demand count for `key`.
    pub async fn count(&self, key: &Key) -> u32 {
        let counts = self.counts.lock().await;
        counts.get(key).copied().unwrap_or(0)
    }
}

impl Default for CollectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
