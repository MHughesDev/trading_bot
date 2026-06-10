//! Rising-edge execution with idempotency deduplication.
//!
//! An automation fires on the **rising edge** of its final condition (false → true).
//! A replayed or recomputed signal with a key that has already been seen is a no-op.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

/// Stable idempotency key for one automation signal.
///
/// A new `signal_epoch` is generated for each distinct false→true crossing;
/// subsequent `true` evaluations at the same epoch produce the same key and are deduped.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IdempotencyKey {
    pub automation_id: Uuid,
    pub instrument_id: String,
    pub stage_id: String,
    pub signal_epoch: u64,
}

/// Tracks per-instrument condition state and deduplicates rising-edge signals.
pub struct RisingEdgeTracker {
    /// Last known condition state per `(automation_id, instrument_id, stage_id)`.
    condition_state: HashMap<(Uuid, String, String), bool>,
    /// Idempotency keys for signals that have already fired.
    seen_keys: HashSet<IdempotencyKey>,
}

impl RisingEdgeTracker {
    pub fn new() -> Self {
        Self {
            condition_state: HashMap::new(),
            seen_keys: HashSet::new(),
        }
    }

    /// Returns `true` iff this evaluation represents a rising edge **and** the key
    /// has not been seen before.
    ///
    /// - `condition = true` while previous state was `false` → rising edge.
    /// - `condition = true` while previous state was already `true` → no edge.
    /// - `condition = false` → reset state; no fire.
    /// - Key already in `seen_keys` → no-op regardless of edge.
    pub fn should_fire(&mut self, key: IdempotencyKey, condition: bool) -> bool {
        let state_key = (
            key.automation_id,
            key.instrument_id.clone(),
            key.stage_id.clone(),
        );
        let was_true = *self.condition_state.get(&state_key).unwrap_or(&false);
        self.condition_state.insert(state_key, condition);

        if condition && !was_true {
            // Rising edge — check idempotency key.
            if self.seen_keys.contains(&key) {
                false
            } else {
                self.seen_keys.insert(key);
                true
            }
        } else {
            false
        }
    }
}

impl Default for RisingEdgeTracker {
    fn default() -> Self {
        Self::new()
    }
}
