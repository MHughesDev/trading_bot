//! Ref-counted registry of running collectors per (venue_id, lane, instrument_id).
//!
//! The registry tracks how many consumers have demanded each stream.  A collector
//! is only spawned when the first subscriber arrives and is stopped when the last
//! one leaves.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::task::JoinHandle;

/// Composite key: (venue_id, lane, instrument_id).
type Key = (Arc<str>, Arc<str>, Arc<str>);

/// Tracks subscriber counts and running task handles.
pub struct CollectorRegistry {
    counts: Arc<DashMap<Key, u32>>,
    handles: Arc<DashMap<Key, JoinHandle<()>>>,
}

impl CollectorRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            counts: Arc::new(DashMap::new()),
            handles: Arc::new(DashMap::new()),
        }
    }

    /// Increment the demand ref-count for `key`.
    ///
    /// Returns `true` when this is the **first** subscriber (count transitions
    /// from 0 → 1) — the caller should spawn the collector.
    pub fn incr(&self, key: Key) -> bool {
        let mut entry = self.counts.entry(key).or_insert(0);
        *entry += 1;
        *entry == 1
    }

    /// Decrement the demand ref-count for `key`.
    ///
    /// Returns `true` when the count reaches zero — the caller should stop the
    /// collector.
    pub fn decr(&self, key: &Key) -> bool {
        if let Some(mut count) = self.counts.get_mut(key) {
            if *count > 0 {
                *count -= 1;
            }
            if *count == 0 {
                drop(count);
                self.counts.remove(key);
                return true;
            }
        }
        false
    }

    /// Store a running task handle for `key`.
    pub fn insert_handle(&self, key: Key, handle: JoinHandle<()>) {
        self.handles.insert(key, handle);
    }

    /// Abort and remove the task handle for `key`.
    pub fn remove_handle(&self, key: &Key) {
        if let Some((_, handle)) = self.handles.remove(key) {
            handle.abort();
        }
    }

    /// Return the current demand count for `key`.
    pub fn count(&self, key: &Key) -> u32 {
        self.counts.get(key).map(|c| *c).unwrap_or(0)
    }
}

impl Default for CollectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}
