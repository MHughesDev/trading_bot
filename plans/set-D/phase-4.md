# Phase 4 — Testing & Conventions

**Completion: 0% (0 / 5 tasks complete)**

**Goal:** Cover the untested complex paths and satisfy the repo's engineering
conventions. **Addresses:** #3, #17, #23, #24, #26 (and #6 semantics)

---

## Tasks

### ☐ 4.1 End-to-end test with seeded ClickHouse — L
**Addresses #23.**
- Stand up ClickHouse (testcontainers or the existing docker-compose), seed
  `market_bars`, run a job `create → … → Completed`, assert results.
- Provide a no-network path by stubbing the collectors so the e2e is hermetic.
- **Files:** `crates/backtest/tests/e2e.rs`.

### ☐ 4.2 Unit tests for `manager` and `sim` — M
**Addresses #24, #6.**
- State-machine transitions, cancellation, persistence/hydrate round-trip.
- The strategy-def → callback bridge: a deterministic EMA-cross over fixture bars
  producing the expected number of orders (also pins signal semantics, #6).
- **Files:** `crates/backtest/src/manager.rs`, `sim.rs` test modules.

### ☐ 4.3 Enforce workspace lint policy on the new crate — S
**Addresses #3.**
- Add `[lints] workspace = true` to `crates/backtest/Cargo.toml`; run
  `cargo clippy --workspace` and `cargo fmt --check`; fix fallout.
- **Files:** `crates/backtest/Cargo.toml` (+ any lint fixes).

### ☐ 4.4 Run the security review — S
**Addresses #17.**
- Run the `/security-review` skill on the diff (outbound network calls; the raw
  SQL, once 1.1 lands) and address findings.

### ☐ 4.5 Project artifacts (specs / ADR / adversarial tests) — M
**Addresses #26.**
- Write a spec for the backtesting mechanism and (with 0.8) the ADR; add an
  adversarial test per mechanism per Invariant 8; add traceability links.
- **Files:** `docs/specs/…`, `docs/adr/…`, tests.

---

## Definition of Done
The full job path is covered by a hermetic e2e test; manager/sim have unit
coverage; workspace lints + fmt pass on the new crate; the security review is
clean; spec/ADR exist with the mandated adversarial tests.
