# Issue #002 — JSON + six heap Strings per envelope

## Summary
| Field | Value |
|-------|-------|
| Severity | High |
| Phase | A |
| Pattern | Serialization |
| Quick Win | No |
| Latency Impact | ~8+ allocations per event; payload 3–5× larger than binary; encode+decode CPU per tick |
| Location | `crates/domain/src/envelope.rs:24-32` |

## Problem
The envelope carries `event_type`, `schema_version`, `lane`, `instrument_id`, `venue_id`, and `source` as owned Strings serialized as JSON on every event. Three of these fields are derivable from context and should not be in the body at all. Every tick through the bus pays 8+ allocations for metadata that is either redundant or static.

## Root Cause
The `Envelope` struct in `envelope.rs` was designed for human-readable logging. All fields are `String` (not interned IDs), and serialization uses `serde_json`, which heap-allocates every string field. Three fields (`event_type`, `schema_version`, `lane`) can be derived from the payload type and routing configuration and need not travel with every event.

## Implementation Plan
### Step 1 — Add `rkyv` to workspace
Add `rkyv = { version = "...", features = ["derive"] }` to the workspace `Cargo.toml`. Relax workspace-level `unsafe_code = "forbid"` to per-crate `deny` (rkyv's zero-copy deserialization requires unsafe).

### Step 2 — Intern instrument/venue/source IDs
Create `InstrumentId(u32)`, `VenueId(u32)`, `SourceId(u32)` newtypes in `crates/domain/src/instrument.rs`. At startup, load the instruments table from Postgres and populate a global intern table mapping string identifiers to u32. All internal code uses u32 IDs; string resolution happens only at API/logging boundaries.

### Step 3 — Redesign the Envelope
Remove `event_type`, `schema_version`, and `lane` from the `Envelope` body — these are derivable from the rkyv type tag and the NATS subject. Replace `instrument_id: String`, `venue_id: String`, `source: String` with `InstrumentId(u32)`, `VenueId(u32)`, `SourceId(u32)`. Fixed envelope header target: ≤ 96 bytes.

### Step 4 — Derive rkyv traits on all payload types
Add `#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]` to `Envelope` and all payload types (`TradePayload`, `BarPayload`, `FeaturePayload`, etc.). Consumers call `rkyv::access::<Archived<Envelope>>()` on the raw bytes — zero copy, zero allocation.

### Step 5 — Quarantine JSON to boundaries
Restrict `serde_json` usage to: REST API request/response, quarantine lane, Parquet raw archive writer. Add a CI check that `serde_json` is not imported in market-lane hot paths.

### Step 6 — Update event-bus publish/subscribe
Update `crates/event-bus/src/publish.rs` and `crates/event-bus/src/lib.rs` to serialize with `rkyv::to_bytes` and deserialize with `rkyv::access` instead of `serde_json`.

## Acceptance Criteria
- [ ] Zero `serde_json` calls on market data lanes (verified by CI grep lint)
- [ ] Fixed envelope header ≤ 96 bytes (verified by `size_of` test)
- [ ] All market-path payloads have rkyv Archive/Serialize/Deserialize derived
- [ ] String intern table populated at startup; no runtime string lookups for instrument IDs
- [ ] JSON still works at REST API and Parquet writer (regression test)

## Files to Change
- `crates/domain/src/envelope.rs` — redesign struct, replace String fields with interned IDs
- `crates/domain/src/instrument.rs` — add InstrumentId/VenueId/SourceId newtypes and intern table
- `crates/event-bus/src/lib.rs` — update subscribe/receive to use rkyv
- `crates/event-bus/src/publish.rs` — update publish to use rkyv serialization
- `Cargo.toml` — add rkyv dependency; relax unsafe_code lint
