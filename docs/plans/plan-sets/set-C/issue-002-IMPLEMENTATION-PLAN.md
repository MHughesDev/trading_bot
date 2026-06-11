# Issue #2 Implementation Plan — Binary Envelope with rkyv

**Companion to:** `issue-002-DESIGN.md`
**Estimated effort:** 30–45 hours across 6 phases
**Status:** Ready to execute

---

## Phase 1: rkyv Derives & Schema Version Removal (6–10 hours)

### Task 1.1: Add rkyv dependency to workspace
- **File:** `Cargo.toml`
- **Action:**
  - Add `rkyv = { version = "0.7", features = ["derive"] }` to workspace dependencies
  - Relax workspace-level `unsafe_code = "forbid"` to `deny` (rkyv's bytecheck requires unsafe)
  - Add SAFETY comment documenting rkyv as approved unsafe dependency
- **Verification:** `cargo tree | grep rkyv` shows dependency across all crates

### Task 1.2: Derive rkyv on all 7 payload types
- **Files:** `crates/domain/src/payloads/*.rs` and `crates/domain/src/lib.rs`
- **Payload types:** `Envelope`, `TradePayload`, `BarPayload`, `QuotePayload`, `FeaturePayload`, `BookPayload`, `HealthPayload`
- **Action per type:**
  - Add `#[derive(Archive, Serialize, Deserialize)]` (from `rkyv::`)
  - Ensure all field types are rkyv-compatible (primitives, vecs, strings all OK)
  - Run `cargo check` to verify no conflicts with existing derives
- **Verification:** `cargo check` succeeds; no compilation errors on domain crate

### Task 1.3: Remove schema_version String field from Envelope
- **File:** `crates/domain/src/envelope.rs`
- **Current state:** `schema_version: String` (redundant; same for all events of a type)
- **Action:**
  - Delete the `schema_version: String` field from Envelope struct
  - Delete the field from `Envelope::new()` constructor
  - Remove schema_version from any serialization tests
  - Add comment: "Schema version is redundant; type tag in rkyv Archive handles versioning"
- **Verification:**
  - `cargo test -p domain` passes
  - Envelope size test: `assert_eq!(std::mem::size_of::<Envelope>(), 96)` (now 96 bytes exactly)

### Task 1.4: Update Envelope::new() and test helpers
- **Files:** `crates/domain/src/envelope.rs`, all test files creating Envelope
- **Action:**
  - Simplify `Envelope::new()` signature (remove schema_version param)
  - Update all test helpers to not pass schema_version
  - Update integration test fixtures in event-bus tests
- **Verification:** `cargo test` passes across all crates; no compilation errors on Envelope construction

---

## Phase 2: Checked-Access API with bytecheck (4–6 hours)

### Task 2.1: Implement rkyv access helpers module
- **File:** `crates/domain/src/access.rs` (new file)
- **Actions:**
  - Create module with two functions:
    - `pub fn access_trusted<T: rkyv::Archive>(bytes: &[u8]) -> &T::Archived` — uses `unsafe { rkyv::access_unchecked(bytes) }` with SAFETY comment (only for in-process ring)
    - `pub fn access_checked<T: rkyv::Archive + rkyv::CheckedArchive>(bytes: &[u8]) -> Result<&T::Archived, bytecheck::Error>` — validates with bytecheck before access
  - Both functions include examples and SAFETY documentation
- **Dependencies:** Add `bytecheck` to domain `Cargo.toml`
- **Verification:** `cargo doc -p domain` shows both functions documented

### Task 2.2: Export access helpers from domain::lib
- **File:** `crates/domain/src/lib.rs`
- **Action:**
  - Export `pub use access::{access_trusted, access_checked}`
  - Ensure availability to consumers as `domain::access_trusted` and `domain::access_checked`
- **Verification:** Consumers in Phase 3 can import without path changes

---

## Phase 3: Collector Encode/Decode Migration (6–8 hours)

### Task 3.1: Identify and audit all 5 collectors
- **Files:**
  - `crates/collectors/src/crypto/kraken.rs`
  - `crates/collectors/src/equity/iex.rs`
  - `crates/collectors/src/web/scraper.rs`
  - `crates/collectors/src/forex/oanda.rs`
  - `crates/collectors/src/crypto/coinbase.rs`
- **Action:** For each collector, find:
  - Where it calls `serde_json::to_string(&payload)`
  - Where it calls `serde_json::from_str::<Payload>`
  - Any intermediate JSON string allocations on hot path
- **Deliverable:** Comment block listing all 5 locations

### Task 3.2: Replace JSON encode with rkyv in collectors
- **Per-collector pattern:**
  - Replace `serde_json::to_string(&payload)` with `rkyv::to_bytes::<_, 2048>(&payload)?`
  - Replace string publish with bytes publish (NATS already supports)
  - Remove `serde_json` import
- **Files to update:** Same 5 collectors above
- **Verification:** Each collector compiles; tests still pass

### Task 3.3: Update event-bus decode to use rkyv
- **File:** `crates/event-bus/src/subscribe.rs` (or wherever decode happens)
- **Current state:** Uses `serde_json::from_slice` on received bytes
- **Action:**
  - Replace with `domain::access_checked::<Envelope>(bytes)?`
  - Add error handling for bytecheck failures (route to invalid message handler)
  - Remove `serde_json` import from hot path
- **Verification:** `cargo test -p event-bus` passes; integration tests with collectors still work

### Task 3.4: Audit and remove serde_json from market lanes
- **Action:** Run `grep -r "serde_json" crates/ --include="*.rs" | grep -E "(kraken|iex|scraper|oanda|coinbase|event-bus|domain)"`
- **Expected:** serde_json remains only in REST API, quarantine lane, Parquet writer (not in hot paths)
- **Fix any violations:** If serde_json found in hot path, migrate to rkyv

---

## Phase 4: Intern Epoch Deterministic Seed (8–12 hours)

### Task 4.1: Implement intern table at process startup
- **File:** `crates/domain/src/intern.rs` (new file)
- **Actions:**
  - Create global static `INTERN_TABLE` with `once_cell`
  - InternerState = `{ instruments: DashMap<String, InstrumentId>, next_id: AtomicU32, epoch_hash: u64 }`
  - Implement `pub fn intern_instrument(name: &str) -> InstrumentId` — lock-free DashMap insert or return existing
  - Implement `pub fn seed_intern_table(instruments: Vec<String>) -> u64` — loads **sorted** instruments, computes xxh3 hash, returns epoch_hash
- **Dependencies:** Add `xxhash-rust = { version = "0.8", features = ["xxh3"] }` to workspace
- **Verification:** Unit test: seed with `["BTC", "ETH"]`, verify deterministic epoch_hash on repeat calls

### Task 4.2: Update collectors to call seed_intern_table at startup
- **File:** `crates/collectors/src/lib.rs` or main collector coordinator
- **Action:**
  - At process startup (before any collector runs), call `domain::intern::seed_intern_table(...)`
  - Load instrument list from Postgres once (already exists as startup config)
  - Store returned epoch_hash in SharedState
- **Verification:** Process starts; no compile errors; epoch_hash printed to logs

### Task 4.3: Stamp NATS header with epoch_hash
- **File:** `crates/event-bus/src/publish.rs`
- **Action:**
  - On publish, add NATS header `X-Epoch-Hash: {epoch_hash}` (u64 as hex string)
  - Header is metadata, not counted in message body
- **Verification:** `cargo test -p event-bus` passes; can inspect header in subscriber test

### Task 4.4: Validate epoch_hash on subscribe
- **File:** `crates/event-bus/src/subscribe.rs` or middleware
- **Actions:**
  - On first message from a publisher, extract `X-Epoch-Hash` header
  - Compare with local epoch_hash (cached, one check per publisher process)
  - If mismatch: route message to quarantine lane (never decode, preserve bytes for audit)
  - Cache decision keyed by publisher identity (from NATS subject)
- **Verification:**
  - Test: seed with mismatched instrument lists, verify quarantine routing
  - Test: matching seeds, verify normal decode
  - Benchmark: header extraction + cache lookup << 1µs (criterion)

### Task 4.5: Document intern table contract
- **File:** `crates/domain/src/intern.rs` (doc comments)
- **SAFETY comment:** InstrumentId(u32) is only valid within process or when epoch_hash matches; cross-process use without header validation is a data-corruption hazard
- **Verification:** Comments present and pass rustdoc build

---

## Phase 5: xtask Lint — No JSON in Hot Paths (2–3 hours)

### Task 5.1: Add lint task to xtask
- **File:** `xtask/src/main.rs` (or new `lint.rs` subcommand)
- **Action:**
  - Create `cargo xtask lint-no-json-hotpath` command
  - Search for `serde_json` imports in market-lane hot paths: `crates/{collectors,event-bus,strategy-runtime}/src/*.rs`
  - Whitelist: `crates/api/src/routes/**` (REST API boundary), `crates/storage/src/parquet/**` (archive writer), quarantine lane
  - On violation: `exit(1)` with error message listing files and line numbers
- **Verification:**
  - `cargo xtask lint-no-json-hotpath` succeeds (no violations)
  - Manually add `use serde_json` to a hot path, re-run, verify failure

### Task 5.2: Integrate lint into CI
- **File:** `.github/workflows/ci.yml` (or equivalent)
- **Action:**
  - Add CI step: `cargo xtask lint-no-json-hotpath` alongside clippy
  - Set to fail the workflow on violation
- **Verification:** Test commit with violation; confirm CI blocks merge

---

## Phase 6: Benchmarks & Metrics (4–6 hours)

### Task 6.1: Create benchmark for serialization overhead
- **File:** `crates/domain/benches/envelope_serde.rs` (new file)
- **Benchmarks:**
  - `bench_json_encode` — `serde_json::to_string(&envelope)`
  - `bench_json_decode` — `serde_json::from_slice(&bytes)`
  - `bench_rkyv_encode` — `rkyv::to_bytes(&envelope)`
  - `bench_rkyv_access` — `access_trusted::<Envelope>(bytes)` (in-process)
  - `bench_rkyv_checked` — `access_checked::<Envelope>(bytes)` (with bytecheck)
- **Metrics captured:** throughput (ops/sec), allocs/op, latency p50/p99
- **Verification:** `cargo bench -p domain` runs; output shows rkyv > 2× throughput, < 50% allocations

### Task 6.2: Memory footprint validation
- **File:** `crates/domain/tests/envelope_size.rs` (new or updated)
- **Tests:**
  - `assert_eq!(std::mem::size_of::<Envelope>(), 96)` — fixed 96-byte header
  - Archived form size check — zero overhead on deserialized form
  - Regression: archived round-trips identically to original
- **Verification:** Tests pass; document expected size in DESIGN.md

### Task 6.3: Integration test — collector → subscriber round-trip
- **File:** `crates/event-bus/tests/rkyv_roundtrip.rs` (new file)
- **Test:**
  - Create sample TradePayload
  - Encode with `rkyv::to_bytes`, publish to NATS
  - Subscriber receives with `access_checked` (NATS path) and `access_trusted` (ring path)
  - Decode results are identical
  - Benchmark roundtrip latency and allocation count
- **Verification:** Test passes; latency < 10µs roundtrip, allocs == 0 on access path

### Task 6.4: Document metrics in DESIGN.md
- **File:** `docs/plans/plan-sets/set-C/issue-002-DESIGN.md` — add results section
- **Content:**
  - Before/after serialization throughput (ops/sec)
  - Envelope size reduction (bytes per event)
  - Allocation reduction per tick
  - p99 latency improvement (subscriber perspective)
  - Intern table startup overhead (milliseconds)
  - Epoch hash validation latency (microseconds, amortized)
- **Verification:** Document includes benchmark results; baseline vs rkyv side-by-side

---

## Success Criteria & Acceptance Tests

### Functional
- [ ] All 7 payload types compile with rkyv derives
- [ ] Envelope size is exactly 96 bytes (no padding overhead)
- [ ] InstrumentId, VenueId, SourceId are u32 newtypes with interning
- [ ] Collectors encode payloads as rkyv bytes (not JSON)
- [ ] Event-bus decode validates epoch_hash on first message; routes mismatches to quarantine
- [ ] REST API still works (JSON encode/decode at boundary, tested)
- [ ] Parquet writer still works (JSON on archive path, tested)

### Performance
- [ ] Serialization throughput: rkyv > 2× faster than serde_json
- [ ] Allocations per event: < 1 on access path (only ringbuffer slot, not serialization)
- [ ] Envelope header: ≤ 96 bytes fixed
- [ ] Payload sizes: 3–5× smaller than JSON encoding
- [ ] Intern table startup: < 100ms (Postgres load + xxh3 hash)
- [ ] Epoch hash validation latency: < 10µs p99 (DashMap lookup + header extract)

### Code Quality
- [ ] No `serde_json` imports in market-lane hot paths (verified by CI)
- [ ] All unsafe code marked with SAFETY comments
- [ ] rkyv derives compile without warnings
- [ ] Clippy clean on all modified crates
- [ ] All tests pass: `cargo test --workspace`

### Documentation
- [ ] issue-002-DESIGN.md updated with final metrics
- [ ] Intern table contract documented (SAFETY comments in code)
- [ ] Epoch hash validation strategy documented (with failure modes)
- [ ] Benchmark results included in DESIGN.md

---

## Estimated Timeline & Dependencies

| Phase | Hours | Dependencies | Blocker Risk |
|-------|-------|--------------|--------------|
| 1: Derives | 6–10 | None | Low (straightforward derive) |
| 2: Checked API | 4–6 | Phase 1 | Low (small module) |
| 3: Collector migration | 6–8 | Phases 1–2 | Medium (5 collectors, testing) |
| 4: Intern epoch | 8–12 | Phases 1–3 | High (cross-process contract) |
| 5: CI lint | 2–3 | Phases 1–4 | Low (xtask scripting) |
| 6: Benchmarks | 4–6 | All phases | Low (validation only) |
| **TOTAL** | **30–45** | Sequential | **Review before Phase 4** |

---

## Rollout Strategy

- **Phases 1–3:** Ship once tests pass (collector migration and decode work end-to-end)
- **Phase 4:** Code review before merge (cross-process contract is critical; needs team alignment)
- **Phase 5:** Enable in CI immediately after Phase 4 (lint enforces no regression)
- **Phase 6:** Run benchmarks on merge, publish in release notes
