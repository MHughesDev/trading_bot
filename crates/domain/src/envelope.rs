//! Binary `EventEnvelope` — the hot-path event wrapper.
//!
//! Replaces the old generic `EventEnvelope<T>` with a compact, non-generic
//! struct that:
//! - Uses `u32` interned IDs instead of `String` fields (zero heap alloc per event).
//! - Stores the payload as rkyv-encoded `Vec<u8>` (binary, not JSON).
//! - Fits in ≤ 96 bytes so it is cache-friendly in the rtrb ring buffers.
//!
//! # Wire format
//!
//! JetStream receives the full rkyv-serialized `EventEnvelope` via the tee
//! task.  The `lane` is carried by the NATS *subject*, not the envelope itself.
//!
//! # Backward compatibility
//!
//! The old multi-timestamp / `Uuid event_id` / `TrustTier` fields are removed.
//! Storage writers and replay consumers obtain instrument/venue strings from
//! the [`crate::interned`] table and use `timestamp_ns` for both event-time
//! and available-time approximations.

use serde::{Deserialize, Serialize};

use crate::instrument::{InstrumentId, SourceId, VenueId};

/// Compact hot-path event envelope.
///
/// All string identity fields have been replaced by 4-byte interned handles;
/// the typed payload is pre-serialized to rkyv bytes.  Use
/// [`crate::interned::instrument_name`] to recover the human-readable names.
#[derive(
    Clone, Debug, Serialize, Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[rkyv(derive(Debug))]
pub struct EventEnvelope {
    pub instrument_id: InstrumentId,
    pub venue_id: VenueId,
    pub source_id: SourceId,
    /// Monotonically increasing per-source sequence counter.
    pub sequence: u64,
    /// Exchange event timestamp in nanoseconds since Unix epoch.
    pub timestamp_ns: i64,
    /// rkyv-encoded payload bytes (TradePayload, BarPayload, etc.).
    pub payload: Vec<u8>,
}

const _: () = assert!(std::mem::size_of::<EventEnvelope>() <= 96);

impl EventEnvelope {
    /// Convenience constructor.
    pub fn new(
        instrument_id: InstrumentId,
        venue_id: VenueId,
        source_id: SourceId,
        sequence: u64,
        timestamp_ns: i64,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            instrument_id,
            venue_id,
            source_id,
            sequence,
            timestamp_ns,
            payload,
        }
    }

    /// Deserialize the payload bytes into a typed value using rkyv.
    pub fn decode_payload<T>(&self) -> Result<T, rkyv::rancor::Error>
    where
        T: rkyv::Archive,
        T::Archived:
            rkyv::Deserialize<T, rkyv::rancor::Strategy<rkyv::de::Pool, rkyv::rancor::Error>>,
    {
        // SAFETY: payload bytes were produced by `rkyv::to_bytes` in this process.
        #[allow(unsafe_code)]
        let archived = unsafe { rkyv::access_unchecked::<T::Archived>(&self.payload) };
        rkyv::deserialize(archived)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interned::{intern_instrument, intern_source, intern_venue};

    #[test]
    fn size_within_96_bytes() {
        assert!(std::mem::size_of::<EventEnvelope>() <= 96);
    }

    #[test]
    fn round_trips_via_serde_json() {
        let env = EventEnvelope::new(
            intern_instrument("BTC-USD"),
            intern_venue("kraken"),
            intern_source("kraken_ws"),
            1,
            1_700_000_000_000_000_000,
            vec![0xDE, 0xAD, 0xBE, 0xEF],
        );
        let json = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(env.instrument_id, back.instrument_id);
        assert_eq!(env.sequence, back.sequence);
        assert_eq!(env.payload, back.payload);
    }
}
