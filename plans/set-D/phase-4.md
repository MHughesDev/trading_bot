# Phase 4 — Testing & Conventions

**Completion: 100% (5 / 5 — 4.1 e2e added as an env-gated live-ClickHouse test; 4.2–4.5 done)**

**Goal:** Cover the untested complex paths and satisfy the repo's engineering
conventions. **Addresses:** #3, #17, #23, #24, #26 (and #6 semantics)

---

## Tasks

### ☑ 4.1 End-to-end test with seeded ClickHouse — L
**Addresses #23.**
- `crates/backtest/tests/e2e.rs` seeds `market_bars` through the real typed
  `insert_collected` path, reads it back through the deduplicated `argMax`
  `load_bars`/`load_features` queries, then runs `run_simulation` over the
  round-tripped bars and asserts the rising-edge crossover places exactly one
  order — a genuine ClickHouse round-trip the in-process tests can't cover.
- **Network-hermetic:** it seeds ClickHouse directly (no collector / REST
  backfill) and simulates in process, so nothing reaches a third-party venue.
- It still needs a live ClickHouse, so it is **gated on
  `BACKTEST_E2E_CLICKHOUSE_URL`**: unset ⇒ the test logs and returns, keeping
  `cargo test` green where the service isn't available (the `sim.rs` hermetic
  bridge test covers the sim path there). Run with
  `BACKTEST_E2E_CLICKHOUSE_URL=http://localhost:8123 cargo test -p backtest --test e2e`.
- **Files:** `crates/backtest/tests/e2e.rs`.

### ☑ 4.2 Unit tests for `manager` and `sim` — M
**Addresses #24, #6.**
- State-machine transitions, cancellation, persistence/hydrate round-trip.
- The strategy-def → callback bridge: a deterministic EMA-cross over fixture bars
  producing the expected number of orders (also pins signal semantics, #6).
- **Files:** `crates/backtest/src/manager.rs`, `sim.rs` test modules.

### ☑ 4.3 Enforce workspace lint policy on the new crate — S
**Addresses #3.**
- Add `[lints] workspace = true` to `crates/backtest/Cargo.toml`; run
  `cargo clippy --workspace` and `cargo fmt --check`; fix fallout.
- **Files:** `crates/backtest/Cargo.toml` (+ any lint fixes).

### ☑ 4.4 Run the security review — S
**Addresses #17.**
- Run the `/security-review` skill on the diff (outbound network calls; the raw
  SQL, once 1.1 lands) and address findings.

### ☑ 4.5 Project artifacts (specs / ADR / adversarial tests) — M
**Addresses #26.**
- Write a spec for the backtesting mechanism and (with 0.8) the ADR; add an
  adversarial test per mechanism per Invariant 8; add traceability links.
- **Files:** `docs/specs/…`, `docs/adr/…`, tests.

---

## Definition of Done
The full job path is covered by a hermetic e2e test; manager/sim have unit
coverage; workspace lints + fmt pass on the new crate; the security review is
clean; spec/ADR exist with the mandated adversarial tests.
