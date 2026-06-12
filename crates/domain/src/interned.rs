//! Global intern table for `InstrumentId`, `VenueId`, and `SourceId`.
//!
//! Interns human-readable name strings to compact `u32` handles that travel
//! through ring buffers and NATS payloads with zero per-event heap allocation.
//!
//! The table is self-seeding: the first call to `intern_instrument(name)` for a
//! given name assigns a new monotonically-increasing ID.  IDs are process-local;
//! cross-process consistency is achieved by calling [`seed_intern_table`] at
//! startup with the same sorted instrument list as all other processes.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use xxhash_rust::xxh3::Xxh3;

use crate::instrument::{InstrumentId, SourceId, VenueId};

struct InternTableInner {
    instrument_by_name: HashMap<String, InstrumentId>,
    instrument_names: Vec<String>,
    venue_by_name: HashMap<String, VenueId>,
    venue_names: Vec<String>,
    source_by_name: HashMap<String, SourceId>,
    source_names: Vec<String>,
    /// xxh3-64 hash of the sorted instrument names at seed time.
    /// Two processes that seed from the same sorted list will produce identical
    /// epoch hashes, guaranteeing cross-process InstrumentId consistency.
    epoch_hash: u64,
}

impl InternTableInner {
    fn new() -> Self {
        Self {
            instrument_by_name: HashMap::new(),
            instrument_names: Vec::new(),
            venue_by_name: HashMap::new(),
            venue_names: Vec::new(),
            source_by_name: HashMap::new(),
            source_names: Vec::new(),
            epoch_hash: 0,
        }
    }
}

static INTERN: OnceLock<Mutex<InternTableInner>> = OnceLock::new();

fn table() -> &'static Mutex<InternTableInner> {
    INTERN.get_or_init(|| Mutex::new(InternTableInner::new()))
}

/// Pre-seed the intern table from a known instrument list and return the
/// deterministic epoch hash.
///
/// Call once at process startup **before** any events flow.  The list is sorted
/// internally so two processes seeding from the same set (in any order) produce
/// the same epoch hash and therefore the same `InstrumentId` assignments.
///
/// The epoch hash is `xxh3_64(sorted_name_0 '\0' sorted_name_1 '\0' …)`.
pub fn seed_intern_table(instruments: &[impl AsRef<str>]) -> u64 {
    let mut sorted: Vec<&str> = instruments.iter().map(|s| s.as_ref()).collect();
    sorted.sort_unstable();

    let mut t = table().lock().expect("intern table poisoned");
    for name in &sorted {
        if t.instrument_by_name.contains_key(*name) {
            continue;
        }
        let id = InstrumentId(t.instrument_names.len() as u32);
        t.instrument_names.push((*name).to_owned());
        t.instrument_by_name.insert((*name).to_owned(), id);
    }

    let mut hasher = Xxh3::new();
    for name in &sorted {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
    }
    let hash = hasher.digest();
    t.epoch_hash = hash;
    hash
}

/// Return the epoch hash set by the most recent [`seed_intern_table`] call.
///
/// Returns `0` if the table has never been seeded (self-seeding mode only).
pub fn epoch_hash() -> u64 {
    table().lock().expect("intern table poisoned").epoch_hash
}

/// Intern an instrument name and return its compact `InstrumentId`.
///
/// Creates a new ID if the name has not been seen before.
pub fn intern_instrument(name: &str) -> InstrumentId {
    let mut t = table().lock().expect("intern table poisoned");
    if let Some(&id) = t.instrument_by_name.get(name) {
        return id;
    }
    let id = InstrumentId(t.instrument_names.len() as u32);
    t.instrument_names.push(name.to_owned());
    t.instrument_by_name.insert(name.to_owned(), id);
    id
}

/// Intern a venue name and return its compact `VenueId`.
pub fn intern_venue(name: &str) -> VenueId {
    let mut t = table().lock().expect("intern table poisoned");
    if let Some(&id) = t.venue_by_name.get(name) {
        return id;
    }
    let id = VenueId(t.venue_names.len() as u32);
    t.venue_names.push(name.to_owned());
    t.venue_by_name.insert(name.to_owned(), id);
    id
}

/// Intern a source name and return its compact `SourceId`.
pub fn intern_source(name: &str) -> SourceId {
    let mut t = table().lock().expect("intern table poisoned");
    if let Some(&id) = t.source_by_name.get(name) {
        return id;
    }
    let id = SourceId(t.source_names.len() as u32);
    t.source_names.push(name.to_owned());
    t.source_by_name.insert(name.to_owned(), id);
    id
}

/// Look up the instrument name for a given `InstrumentId`.
pub fn instrument_name(id: InstrumentId) -> Option<String> {
    let t = table().lock().expect("intern table poisoned");
    t.instrument_names.get(id.0 as usize).cloned()
}

/// Look up the venue name for a given `VenueId`.
pub fn venue_name(id: VenueId) -> Option<String> {
    let t = table().lock().expect("intern table poisoned");
    t.venue_names.get(id.0 as usize).cloned()
}

/// Look up the source name for a given `SourceId`.
pub fn source_name(id: SourceId) -> Option<String> {
    let t = table().lock().expect("intern table poisoned");
    t.source_names.get(id.0 as usize).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_and_lookup_round_trips() {
        let id = intern_instrument("TEST-INTERN-ONLY");
        let name = instrument_name(id).expect("should be present");
        assert_eq!(name, "TEST-INTERN-ONLY");
    }

    #[test]
    fn same_name_returns_same_id() {
        let id1 = intern_instrument("DEDUP-CHECK");
        let id2 = intern_instrument("DEDUP-CHECK");
        assert_eq!(id1, id2);
    }

    #[test]
    fn venue_and_source_intern() {
        let vid = intern_venue("test-venue");
        let sid = intern_source("test-source");
        assert_eq!(venue_name(vid).as_deref(), Some("test-venue"));
        assert_eq!(source_name(sid).as_deref(), Some("test-source"));
    }

    #[test]
    fn seed_is_order_independent() {
        let h1 = seed_intern_table(&["BTC-USD", "ETH-USD", "SOL-USD"]);
        let h2 = seed_intern_table(&["SOL-USD", "BTC-USD", "ETH-USD"]);
        assert_eq!(h1, h2, "epoch hash must be order-independent");
        assert_ne!(h1, 0, "epoch hash must be non-zero for non-empty input");
    }
}
