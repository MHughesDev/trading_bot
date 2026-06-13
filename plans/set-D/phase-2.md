# Phase 2 — Robustness & Operations

**Completion: 100% (6 / 6 tasks complete)**

**Goal:** Make the system resilient under load, flaky networks, and restarts.
**Addresses:** #12, #13, #18, #19, #20, #22

---

## Tasks

### ☑ 2.1 Collector timeouts + retry/backoff — M
**Addresses #12.**
- Build the `reqwest::Client` with connect/read timeouts; wrap page fetches in
  bounded exponential backoff (e.g. 2s/4s/8s/16s); treat exhausted retries as a
  clean phase failure with a clear message.
- **Files:** `crates/backtest/src/collect.rs`, `manager.rs` (client construction).
- **Verify:** test against a mock server returning 429 then 200.

### ☑ 2.2 Job concurrency limit — S
**Addresses #18.**
- Add a config-driven `tokio::sync::Semaphore` (e.g. 2–4 permits) acquired before
  the collect/simulate phases so N creates don't spawn N heavy runs at once.
- **Files:** `crates/backtest/src/manager.rs`, config model.
- **Verify:** test that the (N+1)th job waits while N run.

### ☑ 2.3 Auto-run migrations on startup — S
**Addresses #20.**
- Run `sqlx::migrate!()` in `apps/platform` main (or enforce it in the setup
  hook) so `0011_backtest_runs.sql` is always applied.
- **Files:** `apps/platform/src/main.rs` (or SessionStart hook).
- **Verify:** fresh DB → `backtest_runs` exists after boot.

### ☑ 2.4 Market-holiday calendar for coverage — M
**Addresses #13.**
- Add a per-venue/asset-class holiday source so session markets don't report
  false gaps on holidays/half-days and waste collection attempts.
- **Files:** `crates/backtest/src/gaps.rs` (+ a calendar data source).
- **Verify:** test that a weekday US market holiday isn't counted as expected
  for equities.

### ☑ 2.5 Paginate the list endpoint — S
**Addresses #22.**
- Add `limit`/`offset` (or cursor) to `GET /api/backtests`; cap retained
  in-memory jobs; surface paging in the UI list query.
- **Files:** `crates/api/src/routes/backtests.rs`, `crates/backtest/src/manager.rs`,
  `frontend/src/api/backtests.ts`.

### ☑ 2.6 Decide restart behavior — S
**Addresses #19.**
- Either keep "interrupted on restart" (document as intended) or persist enough
  to resume `Queued` jobs. **Recommended:** document interrupted-by-design now,
  revisit if persistence is needed.
- **Files:** ADR note / `crates/backtest/src/manager.rs` docs.

---

## Definition of Done
Collectors survive transient network failures; concurrent runs are bounded;
migrations apply automatically; session-market coverage respects holidays; the
list endpoint pages; restart behavior is documented or implemented.
