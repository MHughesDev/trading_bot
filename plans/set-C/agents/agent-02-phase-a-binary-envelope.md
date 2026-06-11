# Agent Query — Replace JSON Envelope with rkyv Binary + Interned u32 IDs
## Covers Issues: #2
## Phase: A
## Estimated Effort: 2–3 weeks
## Prerequisites: None (coordinate with #1 — the intern table is shared across both)

---

**How to use this query:** Paste the contents of this file into a new Claude Code session opened in the trading-bot repository root. The agent will implement all fixes listed, verify them, and report completion. Each acceptance criterion has a checkbox — the agent should check them off as they pass.

---

## Background

Every market event in the trading-bot carries an `EventEnvelope` with six owned `String` fields serialized as JSON. Three of these fields (`event_type`, `schema_version`, `lane`) are derivable from context and do not need to be stored per-event at all. The remaining three (`instrument_id`, `venue_id`, `source`) are repeated on every event and could be interned `u32` integers instead of heap-allocated strings. The current design causes 8+ heap allocations per event and a payload 3–5× larger than necessary, creating significant allocation pressure on the hot path.

## Codebase Context

- `crates/domain/src/envelope.rs` — defines `EventEnvelope` with all-String fields (around lines 24–32).
- `crates/domain/src/instrument.rs` — defines instrument-related types; no interned ID types currently exist.
- `crates/domain/src/payloads/trade.rs` — `TradePayload` struct; uses `serde_json` derive.
- `crates/domain/src/payloads/bar.rs` — `BarPayload` struct; uses `serde_json` derive.
- `crates/event-bus/src/publish.rs` — serializes envelopes with `serde_json::to_vec`.
- `crates/event-bus/src/lib.rs` — deserializes envelopes with `serde_json::from_slice`.
- `Cargo.toml` — workspace manifest; `rkyv` and `xxhash-rust` are not yet dependencies.

Current `EventEnvelope` structure (problematic):
```rust
// crates/domain/src/envelope.rs ~line 24
pub struct EventEnvelope {
    pub event_type: String,       // derivable from Rust type
    pub schema_version: String,   // derivable from const
    pub lane: String,             // derivable from NATS subject
    pub instrument_id: String,    // should be u32
    pub venue_id: String,         // should be u32
    pub source: String,           // should be u32
    pub payload: Vec<u8>,
}
```

## Task

### Fix #2 — rkyv zero-copy binary envelope + interned IDs

**Problem:** `EventEnvelope` has six `String` fields. Three are redundant (derivable from context); three should be `u32` intern IDs. `serde_json` is used to encode and decode every market event on the hot path, causing multiple allocations per event and payloads far larger than needed.

**Solution:** Introduce interned `u32` newtypes for `InstrumentId`, `VenueId`, and `SourceId`. Populate an intern table at startup. Replace JSON encode/decode with `rkyv` zero-copy serialization. Remove the three redundant fields from the wire format.

**Implementation steps:**

1. Add to `Cargo.toml` workspace dependencies:
   ```toml
   rkyv = { version = "0.8", features = ["derive"] }
   xxhash-rust = { version = "0.8", features = ["xxh3"] }
   ```

2. Change the workspace lint from `unsafe_code = "forbid"` to `unsafe_code = "deny"` in `Cargo.toml`. Add `#![allow(unsafe_code)]` with an audit comment only in:
   - `crates/domain/src/lib.rs` (rkyv Archive impls require unsafe)
   - `crates/event-bus/src/lib.rs` (zero-copy buffer access requires unsafe)

3. In `crates/domain/src/instrument.rs`, add the interned ID newtypes:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,
            rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
   #[archive(compare(PartialEq), check_bytes)]
   pub struct InstrumentId(pub u32);

   #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,
            rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
   #[archive(compare(PartialEq), check_bytes)]
   pub struct VenueId(pub u32);

   #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,
            rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
   #[archive(compare(PartialEq), check_bytes)]
   pub struct SourceId(pub u32);
   ```

4. Add a global intern table in `crates/domain/src/interned.rs` (new file):
   ```rust
   use std::sync::OnceLock;
   use std::collections::HashMap;
   use super::instrument::{InstrumentId, VenueId, SourceId};

   pub struct InternTable {
       instrument_by_name: HashMap<String, InstrumentId>,
       instrument_by_id:   Vec<&'static str>,
       venue_by_name:      HashMap<String, VenueId>,
       venue_by_id:        Vec<&'static str>,
       source_by_name:     HashMap<String, SourceId>,
       source_by_id:       Vec<&'static str>,
   }

   static INTERN: OnceLock<InternTable> = OnceLock::new();

   pub fn init_intern_table(table: InternTable) {
       INTERN.set(table).expect("intern table already initialized");
   }

   pub fn get_intern() -> &'static InternTable {
       INTERN.get().expect("intern table not yet initialized")
   }
   ```
   Populate the `InternTable` at startup (in `apps/platform/src/main.rs`) by reading known instruments/venues from configuration or Postgres before starting the hot path.

5. Redesign `EventEnvelope` in `crates/domain/src/envelope.rs`:
   ```rust
   #[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
   #[archive(compare(PartialEq), check_bytes)]
   pub struct EventEnvelope {
       // Removed: event_type, schema_version, lane (derive from type tag + NATS subject)
       pub instrument_id: InstrumentId,  // was String
       pub venue_id:      VenueId,       // was String
       pub source_id:     SourceId,      // was String; renamed for clarity
       pub sequence:      u64,           // monotonic per-source counter
       pub timestamp_ns:  i64,           // exchange timestamp, nanoseconds since epoch
       pub payload:       Vec<u8>,       // rkyv-encoded payload bytes
   }
   ```
   Target: `size_of::<EventEnvelope>() <= 96` bytes. Add a compile-time assertion:
   ```rust
   const _: () = assert!(std::mem::size_of::<EventEnvelope>() <= 96);
   ```

6. Derive rkyv traits on all market-path payload types in `crates/domain/src/payloads/`:
   - `TradePayload` — add `rkyv::Archive, rkyv::Serialize, rkyv::Deserialize`
   - `BarPayload` — same
   - `FeaturePayload` — same (if it exists)
   - `QuotePayload` — same (if it exists)
   Keep `serde::Serialize, serde::Deserialize` on these types for the REST API and Parquet writer boundaries.

7. Update `crates/event-bus/src/publish.rs`:
   - Replace `serde_json::to_vec(&envelope)?` with `rkyv::to_bytes::<_, 256>(&envelope)?` (256-byte scratch buffer hint).
   - The NATS payload bytes are now the rkyv archive.

8. Update `crates/event-bus/src/lib.rs` (subscribe/receive path):
   - Replace `serde_json::from_slice::<EventEnvelope>(bytes)?` with:
     ```rust
     let archived = rkyv::access::<ArchivedEventEnvelope, rkyv::rancor::Error>(bytes)?;
     // zero-copy read: archived.instrument_id, archived.timestamp_ns, etc.
     ```
   - For payload inspection, deserialize only when needed: `rkyv::deserialize::<EventEnvelope, _>(archived)?`.

9. Quarantine `serde_json` — it may only appear in:
   - `crates/api/` — REST API handlers (request/response bodies)
   - Storage writer's raw-archive path (Parquet/ClickHouse JSON column)
   - Any logging/debug utilities
   Add a comment block in `crates/event-bus/src/lib.rs`: `// serde_json is intentionally absent from this crate — use rkyv for market events`.

**Acceptance test:**
- Add a compile-time test: `const _: () = assert!(std::mem::size_of::<EventEnvelope>() <= 96);`
- Add an integration test that round-trips an `EventEnvelope` through rkyv and verifies field equality.
- Run `cargo grep "serde_json" crates/event-bus/ crates/collectors/` and confirm zero results on market-lane paths.
- Verify the intern table is populated before the hot path starts (add an assertion in `apps/platform/src/main.rs`).

## Overall Acceptance Criteria
- [ ] Zero `serde_json` calls on market data lanes (CI grep: `grep -r "serde_json" crates/event-bus/` returns zero)
- [ ] `size_of::<EventEnvelope>() <= 96` enforced by a compile-time assertion
- [ ] All market-path payloads (`TradePayload`, `BarPayload`, etc.) have rkyv Archive/Serialize/Deserialize derived
- [ ] Intern table (`InstrumentId`, `VenueId`, `SourceId`) populated at startup; no per-event String lookups for IDs
- [ ] JSON still works at REST API layer and Parquet writer (serde derives kept on payload types)
- [ ] `cargo test` passes (all envelope round-trip tests pass)
- [ ] `cargo build --release` succeeds

## Files to Touch
- `crates/domain/src/envelope.rs` — redesign EventEnvelope; remove String fields; add rkyv derives; add size assertion
- `crates/domain/src/instrument.rs` — add InstrumentId, VenueId, SourceId newtypes with rkyv derives
- `crates/domain/src/interned.rs` (new) — InternTable and global OnceLock
- `crates/domain/src/lib.rs` — export new types; add `#![allow(unsafe_code)]`
- `crates/domain/src/payloads/trade.rs` — add rkyv derives
- `crates/domain/src/payloads/bar.rs` — add rkyv derives
- `crates/event-bus/src/lib.rs` — switch deserialization to rkyv; add `#![allow(unsafe_code)]`
- `crates/event-bus/src/publish.rs` — switch serialization to rkyv
- `apps/platform/src/main.rs` — populate intern table at startup before hot path starts
- `Cargo.toml` — add `rkyv` and `xxhash-rust` workspace dependencies
