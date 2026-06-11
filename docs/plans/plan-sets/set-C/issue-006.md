# Issue #006 — UUID v5 (SHA-1) + two format! allocations per tick for dedup identity

## Summary
| Field | Value |
|-------|-------|
| Severity | Medium |
| Phase | C |
| Pattern | Hashing |
| Quick Win | No |
| Latency Impact | 2 heap allocs + SHA-1 (~hundreds of ns) per trade |
| Location | `crates/domain/src/ids.rs:37-58`, `crates/collectors/src/crypto/kraken.rs:108` |

## Problem
Deterministic event identity is implemented by `format!`-ing a dedup key string then SHA-1-hashing it, once per trade, before the event reaches the bus. SHA-1 is cryptographic-quality overkill for a dedup key, and the two `format!` allocations are in the hot collector normalize path.

## Root Cause
`ids.rs` uses UUID v5, which is defined as SHA-1 over a namespace UUID concatenated with a name string. The name string is constructed with `format!("{venue}:{symbol}:{timestamp}:{price}")` — two allocations plus SHA-1 computation per trade.

## Implementation Plan
### Step 1 — Remove event_id computation from collector normalize()
Strip the `event_id` / dedup computation out of every collector's `normalize()` function. Collectors return payloads without an id field populated (or with a placeholder).

### Step 2 — Move identity computation to the storage-writer boundary
The storage writer is the only place that needs a dedup key. Compute identity once, at the point of write.

### Step 3 — Replace SHA-1 with xxh3_128
In `ids.rs`, replace the UUID v5 / SHA-1 algorithm with `xxh3_128` over packed binary fields (venue_id as u32, instrument_id as u32, timestamp as i64, price as u64 bits). No `format!`, no SHA-1. Store as `Uuid::from_u128(hash)`.

### Step 4 — Pack fields directly
Build the hash input as `[u8; 24]` (4 + 4 + 8 + 8 bytes) on the stack. Pass the slice to `xxh3_128`. Zero heap allocation.

### Step 5 — Update all collector normalize() functions
Audit all collectors and confirm none call the old `compute_event_id` / UUID v5 path.

## Acceptance Criteria
- [ ] Zero allocations and zero SHA-1 hashing in any collector `normalize()` function
- [ ] Event identity computed at storage-writer boundary only
- [ ] xxh3_128 used instead of UUID v5 / SHA-1
- [ ] Hash input packed as fixed-size byte array (no `format!`)
- [ ] All collector normalize() functions pass clippy with no allocation warnings on dedup path

## Files to Change
- `crates/domain/src/ids.rs` — replace UUID v5 with xxh3_128 over packed binary
- `crates/collectors/src/crypto/kraken.rs` — remove dedup computation from normalize()
- All other collector normalize() functions (alpaca_data, tradovate, tradier, kalshi, oanda, reddit, scraper)
