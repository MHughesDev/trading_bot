# Phase 2 — Live P&L, Manual Orders & Shared Data Layer

**Completion: 100% (3 / 3 — 2.3 on 2026-09-15; 2.1 and 2.2 on 2026-10-03, track PROD)**

**Goal:** Build the shared position/mark/P&L data layer once, then use it to
turn on the live dashboard rollup and paper-mode manual orders.
**Addresses:** #12, #13. **Depends on:** Phase 1 (real `user_id`).

> **Shared dependency:** items #13 (dashboard rollup) and #12 (manual orders)
> both need position + mark-price data. The hot-path stage-4 dummy
> `GateContext` (`hot_path.rs:184`, handled in 5.4) is the same gap from the
> automated entry point. Build the data layer once in 2.1.

---

## Tasks

### ☑ 2.1 [DONE 2026-10-03 — without Redis, by ADR] Shared trading-data layer in `AppState` — S–M — **prerequisite**

> **2026-10-03 (track PROD).** **ADR-E-2.1:** the shared layer is the paper
> engine's mark board (fed by the live collectors, with observation times) plus
> `AppState::instrument_meta` (`InstrumentMetaCache`) and the `instruments`
> table — not a Redis handle. Rejected alternative: adding Redis for marks.
> Every consumer that exists reads the in-process board already (the order gate
> since 2.3, the hot path since 5.4, and now the live rollup in 2.2), so a Redis
> copy would be a second mark source that could disagree with the one orders are
> priced against. Locked decision 4's substance — marks surfaced when missing or
> stale, never zeroed — is kept (see 2.2). Revisit only when a process other
> than the platform needs marks.
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

### ☑ 2.2 [DONE 2026-10-03] Live dashboard P&L rollup — M

> **2026-10-03 (track PROD).** `GET /api/dashboard/rollup?mode=LIVE` now reads
> the **caller's** `pnl_lots`/`pnl_closes` (`load_ledger(pg, token.user_id(), …)`,
> every predicate carries `user_id` — the rollup now uses its `BearerToken`),
> joins `instruments` for venue/asset class, values open lots at the platform
> mark board, and returns `mark_coverage`: instruments `missing` a mark, `stale`
> (> 300 s), `unconverted` (non-USD quote) and `unreadable_lots` (no stored side)
> are listed and left out of unrealized — never zeroed silently. Short lots are
> now valued correctly (`compute_rollup` used `mark − open` for every lot).
> Migration **0154** adds `pnl_lots.side` (nullable, `CHECK in ('buy','sell')`).
> Today `orders.execution_mode` is `CHECK = 'paper'`, so a live rollup is a
> computed zero with complete coverage. Tests: `routes::dashboard::tests` (4 —
> seeded non-zero rollup incl. a short, missing/stale/unconverted/unreadable
> surfaced, another user's lots excluded, the route takes `BearerToken`).
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

### ☑ 2.3 Paper-mode manual order placement — M — **DONE 2026-09-15**
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
- **Verified by:** `hot_path::tests::the_rate_limit_fires_when_breached`,
  `..::the_daily_loss_limit_fires_on_a_real_loss`,
  `..::an_instrument_without_venue_increments_is_refused`,
  `..::an_instrument_without_a_mark_is_refused`. The zero-position bypass
  cannot recur because `GateContext` no longer has a field that can be
  defaulted: every input is `Measured<T>` and `Unavailable` rejects.
- Rate counters are not “wired” by the caller — the **gate owns them**
  (`risk::rate`), keyed by a caller-chosen scope. A call site can choose the
  scope; it can no longer choose the count.
- 2.1’s data layer landed in the shape this task needed: `AppState` carries
  an instrument-metadata cache rather than a Redis handle, because the mark
  source for paper is the in-process paper engine’s mark board, not Redis.

---

## Definition of Done
The live dashboard shows real ledger-derived P&L; paper-mode manual orders flow
through a real risk-gate context and execute (or are correctly rejected); and
the position/mark/registry data layer is available in `AppState` for reuse by
the hot path (5.4). Live manual orders remain explicitly deferred to broker
availability.
