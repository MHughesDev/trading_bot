# Phase 1 — Correctness & Security

**Completion: 0% (0 / 5 tasks complete)**

**Goal:** Close the real correctness and security gaps before the system is used
against live data. **Addresses:** #8, #10, #11, #15, #16, #21

---

## Tasks

### ☐ 1.1 Parameterize the collected-bar insert — M
**Addresses #11.** Remove hand-built SQL string interpolation.
- Replace the raw `INSERT … VALUES (…)` in `store.rs::insert_collected` with the
  `clickhouse` crate's row-based `client.insert("market_bars")` +
  `#[derive(Row, Serialize)]` (mirroring `crates/storage/src/clickhouse/bars.rs`).
- Keep `numeric()` only as a defense-in-depth assertion.
- **Files:** `crates/backtest/src/store.rs`.
- **Verify:** test inserts a batch into a test ClickHouse and reads it back;
  existing `numeric`/`sql_escape` unit tests stay green or are removed with the
  raw path.

### ☐ 1.2 Real precision from instrument metadata + non-panicking constructors — M
**Addresses #8, #21.**
- Look up tick/lot size and precision from the `instruments` Postgres table
  (the `domain::Instrument` type) instead of inferring decimal scale from data;
  thread real precisions into `sim::run_simulation`.
- Switch nautilus `Price::from` / `Quantity::from` / `Money::from` to the
  checked/`new_checked` / `try_*` constructors so malformed input returns an
  error rather than panicking inside `spawn_blocking`.
- **Files:** `crates/backtest/src/sim.rs`, `manager.rs` (instrument lookup).
- **Verify:** unit test with a 0-dp (JPY-style) and an 8-dp crypto instrument.

### ☐ 1.3 Fix Binance symbol mapping — S
**Addresses #10.** Stop silently proxying `-USD` → `USDT` (a different market).
- Require the exact venue symbol, map via the `instruments` table, or refuse
  with a clear error when no exact market exists.
- **Files:** `crates/backtest/src/collect.rs`.
- **Verify:** test asserts `BTC-USD` is no longer rewritten to `BTCUSDT`
  implicitly.

### ☐ 1.4 Validate collector support at create time — S
**Addresses #15.**
- In `routes/backtests.rs::create_backtest`, reject (422) asset-class/timeframe
  combinations with no collector when `auto_collect` is on, with a clear message
  — don't let the job reach `CollectingData` only to fail there.
- Trim the create-form options to supported classes (or label unsupported ones).
- **Files:** `crates/api/src/routes/backtests.rs`, `frontend/.../CreateBacktestDialog.tsx`.
- **Verify:** request for `option` returns 422.

### ☐ 1.5 User-scope backtests — M
**Addresses #16.**
- Add a `user_id` column to `backtest_runs`; thread the authenticated user from
  `BearerToken` into create/list/get/stop/delete; filter lists by user.
- Compatible with the M-17 placeholder until real sessions land — just stops
  cross-user visibility/control.
- **Files:** new migration, `crates/backtest/src/manager.rs`,
  `crates/api/src/routes/backtests.rs`.
- **Verify:** two tokens see disjoint run lists.

---

## Definition of Done
No raw SQL string-building remains; precision comes from instrument metadata;
malformed values error instead of panicking; unsupported requests are rejected
up front; runs are scoped to their creator.
