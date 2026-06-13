# Phase 2 — Live P&L, Manual Orders & Shared Data Layer

**Completion: 0% (0 / 3 tasks complete)**

**Goal:** Build the shared position/mark/P&L data layer once, then use it to
turn on the live dashboard rollup and paper-mode manual orders.
**Addresses:** #12, #13. **Depends on:** Phase 1 (real `user_id`).

> **Shared dependency:** items #13 (dashboard rollup) and #12 (manual orders)
> both need position + mark-price data. The hot-path stage-4 dummy
> `GateContext` (`hot_path.rs:184`, handled in 5.4) is the same gap from the
> automated entry point. Build the data layer once in 2.1.

---

## Tasks

### ☐ 2.1 Shared trading-data layer in `AppState` — S–M — **prerequisite**
The compute is done and tested (`api/src/rollup/compute_rollup`,
`risk/src/gate.rs`); only the data plumbing is missing.
- Add a **Redis handle** to `AppState` (`state.rs` has none today), constructed
  at platform startup, for marks (`RedisCache::get_latest`,
  `storage/src/redis.rs:37`).
- Add **instrument-registry access** for tick/lot size, venue mapping, and
  active/session/halt state.
- Confirm `user_id` extraction is available on these handlers (from Phase 1).
- **Locked decision 4:** the canonical mark is **trade-last from Redis**;
  missing/expired marks are **surfaced** to the user, never silently zeroed.
- **Files:** `crates/api/src/state.rs`, platform startup wiring.
- **Verify:** `AppState` exposes Redis + registry; a smoke test reads a mark.

### ☐ 2.2 Live dashboard P&L rollup — M
**Addresses #13 (CL dashboard.rs).** `AccountMode::Live` returns an all-zero
`RollupResponse` (`dashboard.rs:58-65`). P&L is computed from the **ledger**
(`pnl_lots`/`pnl_closes`, migration 0008), not the broker — so it's achievable
now, independent of any live broker adapter (the in-tree comment overstates the
broker dependency).
- Live branch: `SELECT` the user's `pnl_lots` + matching `pnl_closes`; build
  `marks` from Redis per open-lot instrument; build `venue_map` from the
  registry; call the tested `rollup_from_slices`. Extract `user_id` from the
  Phase-1 extractor.
- Handle non-USD `usd_rate` correctly; surface mark staleness rather than
  silently dropping unrealized P&L.
- **Files:** `crates/api/src/routes/dashboard.rs:54-66`.
- **Verify:** seeded lots/closes + marks produce a correct non-zero rollup;
  missing-mark instruments are surfaced, not silently zeroed.

### ☐ 2.3 Paper-mode manual order placement — M
**Addresses #12 (CL orders.rs).** The handler builds a valid `OrderIntent` then
unconditionally returns **503** (`orders.rs:104-118`) because position/mark were
unwired — a deliberate fail-closed guard against zero-position over-leverage.
- For **paper mode** (achievable now, no broker): read `current_position` from
  `paper_engine.position(instrument_id)`, `market_price` from
  `paper_engine.mark(instrument_id)`, tick/lot/active/session/halt from the
  registry, `daily_loss_usd` from the paper account snapshot. Build a **real**
  `GateContext`, call `risk_gate.check`, then `execution.submit`. Map rejections
  through the already-written `risk_rejection_response` (remove its
  `#[allow(dead_code)]`).
- Wire real per-instrument **rate-limit counters** (passing `0,0` disables rate
  limiting).
- **Locked decision 5: paper-only first cut.** Defer live manual orders until a
  broker adapter + position feed exist; gate on `mode` with a clear
  "live broker not available" response.
- **Files:** `crates/api/src/routes/orders.rs:31,104-118,154`.
- **Verify:** a paper order within limits fills; one breaching position/rate
  limits is rejected with the mapped status; the zero-position bypass cannot
  recur.

---

## Definition of Done
The live dashboard shows real ledger-derived P&L; paper-mode manual orders flow
through a real risk-gate context and execute (or are correctly rejected); and
the position/mark/registry data layer is available in `AppState` for reuse by
the hot path (5.4). Live manual orders remain explicitly deferred to broker
availability.
