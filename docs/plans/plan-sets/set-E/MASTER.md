# Production-Readiness — Stub, Placeholder & Tech-Debt Master Plan — Set E

**Completion: 73% (22 / 30 fully done · 4 partial · 1 blocked-human · 2 superseded by Set U · 1 open; 4.5 deferred-by-design, not counted)
— re-verified 2026-09-15, track 16 (2.3, 5.4), then track PROD 2026-10-03 waves 1–2.
DONE: 0.1–0.5, 1.1–1.6, 2.1–2.3, 3.1, 3.2, 3.3, 3.6, 4.1, 5.3, 5.4, 5.5.
PARTIAL: 1.7 (human security review), 3.5 (live submit blocked), 5.1, 5.2 (each
names the one missing line). BLOCKED:HUMAN: 3.4. SUPERSEDED BY SET U: 4.2, 4.4.
OPEN: 4.3. Future-scope items are not counted.**

> **Untracked findings, status 2026-10-03 (track PROD):**
> **#3 order audit trail — DONE.** `OrderJournal::record_audit` (required, no
> default) writes every `AuditEntry` from inside `ExecutionEngine::submit` and
> `sync_order` to `order_audit` (migration 0152: append-only by trigger for every
> role, `app_role` SELECT+INSERT only). The first entry is written fail-closed
> before the order is recorded. Tests: `order_audit_trail` (3 + 1 PG, incl.
> UPDATE/DELETE refused and an `app_role` insert); mutation-checked (dropping the
> post-submit write fails `a_filled_order_leaves_its_whole_trail_in_the_journal`).
> **#4 kill switch persists — DONE (verified).** `routes/trading.rs` writes
> `monitor.kill_switch` and `global_risk_config.trading_enabled` before tripping
> in memory; `monitor/tests/fail_closed_drill.rs::a_trip_survives_a_restart_and_the_boot_path_agrees`.
> **#5 durable paper engine — PARTIAL.** `execution::paper::persist`
> (`restore`, `save_dirty`, `spawn_saver`) + migration 0153 `paper_account_state`;
> per-account dirty flags set under the lock, a write only when state changed,
> an unknown `state_version` refuses boot rather than reseeding. Tests:
> `paper_state_durable` (2 PG: cash, positions, realized P&L, fees, ledger and the
> gate's daily-loss input identical across a simulated restart) + 3 engine unit
> tests. **Remaining:** two lines in `apps/platform/src/main.rs` (track report).
> Resting paper orders are not restored (documented in 0153).
> **#8 mutex-poison panics — PARTIAL.** Every `.lock().expect(...)` in
> `paper/engine.rs` (17 account sites, 3 FIFO sites) replaced: reads recover the
> guard, new orders on a poisoned account are refused with
> `PaperTradeError::AccountUnavailable`, `reset_account` clears the poison.
> Test: `a_poisoned_account_refuses_orders_and_still_answers_reads`. `risk/gate.rs`
> was already fixed. **Remaining:** `routes/strategies.rs:198,524`
> (`strategy_store.lock().expect`) are Set U territory and untouched.
> **#9 inbound side fails open — DONE (wave 2: all five venues).** `alpaca.rs`
> `domain_side`/`domain_order_type` return `Result`; unknown text is a
> `BrokerError`, never `Sell`/`Market`. Wave 2 fixed the same shape in
> `venues/tradier.rs` (`side.contains("buy")`, type fell through to `Limit`),
> `kalshi.rs` (`== "yes"` else Sell), `oanda.rs` (a missing/garbled `units`
> defaulted to **Buy**; an unknown state reported `New`) and `tradovate.rs`
> (`== "Buy"` else Sell, type to `Limit`), each with a pure parser and tests
> (`inbound_tests` in each file). Also: the three `query_open_orders` that
> returned `Ok(vec![])` — "no open orders" to a reconciler — now return an error
> naming that listing is unimplemented, and Coinbase's strict parser (3.5) was
> built the same way. **#10 (0x `submit` fabricating an id) — fixed:** it refuses.

> Future-scope items (TigerGraph, Milvus) live in [future-scope.md](future-scope.md)
> and are **not** counted in the primary total.

---

## ⚠️ Re-audit, 2026-09-15 — read this before planning any Set E work

The 2026-06-13 sweep is three months and a whole ML platform old. All 30 items
were re-verified against the current code by the plan-hygiene track. **The
headline number barely moved — 5/30 → 6/30 — and that is the finding.** Phases
3, 4 and 5.1–5.4 are untouched since the sweep; Phase 1 moved from 0% to roughly
half-built *by substance* while **no Phase 1 task fully meets its acceptance
criteria**, which is exactly the failure mode this plan set exists to name.

The old count was stale in both directions: 5.5 (MCP tests) landed as a side
effect of other work and was never ticked, and 1.3 was substantially built
without being claimed.

| Verdict | Count |
|---|---|
| **DONE** | **6** — 0.1, 0.2, 0.3, 0.4, 0.5, 5.5 |
| **PARTIAL** | **7** — 1.1, 1.2, 1.3, 1.4, 1.5, 2.1, 2.3 |
| **OPEN** | **17** — 1.6, 1.7, 2.2, 3.1–3.6, 4.1–4.4, 5.1–5.4 |
| OBSOLETE / UNKNOWN | 0 / 0 |

### The answer Stage 3 needs: what blocks unattended paper trading

ROADMAP Stage 3 folds Set E in because these items *"become load-bearing the
moment something runs unattended against a broker."* **Ten of them do.** Ordered
by how much damage a week of unattended running would do, each with the failure
mode:

| # | Task | Failure mode if a paper fleet runs a week unwatched |
|---|---|---|
| **1** | **5.4** hot-path stage 3 | `POST /api/strategies/{id}/start` returns 200 and registers the instance, but **nothing ever calls `dispatch`** (`hot_path.rs:333` is `Option::None`; `InstanceManager::dispatch` has no caller outside tests). Zero intents forever, no error. **Total silent outage of the product's core function.** |
| **2** | **5.1** metrics | `crates/observability/src/metrics.rs:1-17` are empty-body stubs; no `metrics`/`prometheus` dependency, no `/metrics` route, **zero call sites**. Nothing to scrape, nothing to alert on. **This is the item that turns every other row here from "a bug" into "an un-diagnosable incident."** |
| **3** | **2.3** residual | `routes/orders.rs:208-217` passes order counters `0, 0` and `daily_loss_usd = Decimal::ZERO`, so `RateLimitExceeded`, `RateLimitPerSecondExceeded` and `DailyLossLimitExceeded` **can never fire**. A looping strategy or a retrying client submits unbounded orders with no brake. |
| **4** | **4.4** exit nodes | `toDefinition.ts:292-294` drops exit rules on save with a **warning only**. A strategy the operator designed with a stop-loss is persisted without it and runs unprotected all week. |
| **5** | **4.2** `PercentOfBalance` | `intents.rs:31` returns `None` at runtime, while the builder happily saves percent-of-equity sizing. Every signal produces **no order and no error** — indistinguishable from "no signals fired". |
| **6** | **1.3** `/ws/live` | `ws/live.rs:33-38` takes `?token=` and uses **the raw string as the user id** if non-empty — no session lookup, no expiry. The surviving M-17 hole: anyone reaching the port reads another tenant's frames. |
| **7** | **1.4** automations | `routes/automations.rs:27` still `const DEV_USER: Uuid = Uuid::nil()`, and `storage/automation.rs:87,114,124` `list`/`set_armed`/`delete` carry **no `user_id` predicate**. Any authenticated caller can arm or **disarm** anyone's automation. |
| **8** | **2.2** live P&L | `dashboard.rs:54-65` still returns the hard-coded all-zero rollup for `AccountMode::Live`. The operator's only P&L surface reports \$0 instead of erroring — **a losing week looks like a flat week.** |
| **9** | **1.5** `dev-local` | `frontend/src/lib/api.ts:16` and `AppLayout.tsx:50` still fall back to `Bearer dev-local`. It is the credential that makes row 6 reachable from an unauthenticated browser tab. Also diverges from locked decision 1: bearer token in `localStorage`, no cookie, no CSRF. |
| **10** | **1.1** sessions | `migrations/0013_auth.sql` stores `sessions.token` as **cleartext `TEXT PRIMARY KEY`**, not the specified SHA-256 `token_hash BYTEA`; no `last_seen_at`, no sliding expiry, no sweep. A DB dump or a logged row is a full-authority bearer credential, including `POST /api/trading/resume`. |

**Explicitly NOT blocking** — 1.2, 1.6, 1.7, 2.1, 3.1–3.6, 4.1, 4.3, 5.2, 5.3.
These are ordinary debt. The reasoning matters: **1.7's bind guard fails
*closed*** (loopback-only is safer than the flip it asks for, so leaving it is
correct); the Tradier/Tradovate/Coinbase/0x adapters all fail loudly or are
simply unreachable (**only `PaperBroker` is ever constructed** —
`apps/platform/src/main.rs:151`); compaction and `OrderBookBuilder` are inert
no-ops with no consumer.

### Ten things wrong in the code that no Set E item covers

Found during this audit. **None was fixed here.** Several are more dangerous
than items already in the plan, and the first four are, jointly, the reason
"unattended" is not currently a safe word for this platform.

1. **Nothing ever writes an order or a fill to the database.** A repo-wide grep
   for `INSERT INTO orders` / `INSERT INTO fills` returns **zero hits**. The
   `orders` and `fills` tables from `migrations/0003` are permanently empty, so
   `GET /api/orders/{id}` (`routes/orders.rs:255-262`, selecting on
   `idempotency_key`) **can never succeed**.
2. **Order idempotency does not survive a restart.** `ExecutionEngine`'s dedup is
   an in-memory bounded `VecDeque` of 50,000 keys behind a `Mutex`
   (`execution/src/lib.rs:69`, `fills.rs:17`). Its own doc comment at
   `fills.rs:38-41` claims *"the DB `fills` table's UNIQUE constraint provides
   the durable idempotency guarantee"* — **and nothing writes that table**, per
   item 1. The durable guarantee does not exist; a restart mid-week re-applies
   any redelivered fill.
3. **The order audit trail is built and then thrown away.**
   `ExecutionEngine::submit` (`execution/src/lib.rs:85-109`) constructs
   `AuditEntry::state_transition` and `AuditEntry::broker_submit` and returns
   them in `SubmitResult.audit`; `routes/orders.rs:222-236` **ignores the field**,
   and no migration creates an order-audit table. There is no durable record
   that any order was ever placed.
4. **The kill switch does not persist when tripped.**
   `risk/src/kill_switch.rs:26-36` flips only an `AtomicBool`;
   `POST /api/trading/kill` (`routes/trading.rs:19-28`) never writes
   `global_risk_config.trading_enabled`, which boot *does* read
   (`main.rs:483-499`). **A kill switch tripped on Monday is silently reset by
   any restart.** The doc comment says persistence is "the caller's
   responsibility" — no caller does it. This directly undercuts ROADMAP Stage
   3's exit criterion *"a kill-switch drill stops order flow within one
   cadence"*.
5. **The paper engine is entirely in-memory and non-durable.**
   `PaperTradingEngine` holds accounts, positions, cash and marks behind
   `Mutex`es with `snapshot()` and no save/load (`execution/src/paper/engine.rs`).
   A restart silently resets positions, cash and realized P&L to seed state.
   Nothing in Set E covers paper-state durability, and Stage 3 is *made of*
   paper state.
6. **Armed automations are dead code.** `storage::automation::armed_automations`
   (`storage/src/automation.rs:98`) **has no caller**; the only boot-time touch is
   `disarm_all_automations` (`main.rs:282`). `POST /api/automations/{id}/arm`
   returns success and starts a data pipeline, and then **nothing ever evaluates
   the spec**. This is a *second* silent path, distinct from the hot path that
   item 5.4 covers.
7. **No graceful shutdown.** `main.rs:470-478` is `axum::serve(listener,
   router).await` with no `with_graceful_shutdown` and no ctrl-c/SIGTERM handler
   anywhere. In-flight submits and the tee / bar-persist tasks are killed
   mid-flight.
8. **Mutex-poisoning panics on the order and fill paths.** `execution/src/lib.rs:159`
   `.expect("fill_processor lock")`, `risk/src/gate.rs:196,224`
   `.expect("seen_keys lock")` (**on every order approval**),
   `paper/engine.rs:556,567,586,606,696,784,792,963`, and
   `routes/strategies.rs:238,284`. One panic under any of those locks poisons it
   permanently: every subsequent order then panics the handler task rather than
   returning an error, **and the process keeps running looking healthy**.
9. **Inbound order side fails open to `Sell`.** `execution/src/alpaca.rs:99-105`
   and `execution/src/lib.rs:214-220`:
   `if s == "buy" { Side::Buy } else { Side::Sell }`. Outbound mapping is correct,
   but on the inbound path any unrecognised, empty, truncated or differently-cased
   value silently becomes a **short**. In a system that reconstructs positions and
   P&L from broker records, a parse glitch turning a long into a short is the
   wrong direction to fail.
10. **`zerox.rs:79-141` `submit` fabricates a broker order id without
    broadcasting anything** — it returns `Ok(format!("0x:{}", quote.to))`, the 0x
    *contract address*, identical for every order, so it is not even a unique key.
    Task 0.2 fixed `query_order` and left `submit` claiming success for an order
    that was never sent. Harmless only because `ZeroXBroker` is never constructed.
    Also `GET /api/dashboard/rollup` (`routes/dashboard.rs:35-38`) is **the only
    handler in `routes/` with no `BearerToken`** — read-only, but it is the P&L
    surface.

**Recommendation for Stage 3 planning:** the ten blocking items above plus
untracked findings 1–8 are the real Stage-3 prerequisite list, and it is
**eighteen items, not ten**. Findings 1–5 in particular (no order/fill
persistence, no durable idempotency, no order audit trail, a non-persisting kill
switch, a non-durable paper engine) are not "tech debt" — they are the absence of
the substrate Stage 3's exit criteria are written against.

## Overview

Set E consolidates every unfinished, stubbed, stale, or placeholder code path
found in a repository-wide sweep (2026-06-13) into one sequenced plan. It
supersedes and absorbs the tracking table in `/COMPLACENCY_LOG.md` (v2, 21 open
items) and adds findings that log missed:

- The frontend ships a **complete cookie-session auth UX** (`authApi`,
  `useAuthStore`, `LoginPage`, `SignUpPage`) calling `/auth/*` routes that **do
  not exist** in the Rust API — the login flow is dead today and the SPA falls
  back to a hardcoded `dev-local` bearer token.
- `BearerToken` identity is represented **three different ways** across routes
  (UUIDv5-of-token in backtests, raw token string in `streams.rs:48`,
  `Uuid::nil()` DEV_USER in automations) — unifying it is itself cross-cutting.
- Frontend strategy-builder gaps (`toDefinition.ts`): SMA/ATR, AND/OR
  conditions, exit rules, and percent-of-equity sizing are accepted in the UI
  but warned/dropped on save.
- `PercentOfBalance` / `RiskUnit` sizing are parse-only in the **shared**
  `strategy-runtime` (non-executable live *and* in replay).
- Hot-path **stage-3 strategy evaluation is a `None` placeholder** — no intents
  are ever produced by the ring pipeline.
- `crates/graph` (TigerGraph) and `crates/semantic` (Milvus) compile but have
  **zero functional consumers**, yet their heavyweight containers run by default.

Two findings the log under-rated are promoted here to **safety/correctness**:

- `reconciliation/freshness.rs` compares **UTC wall-clock** against venue-local
  session times, so the kill-switch staleness watchdog **silently fails to fire
  during live US equity/options hours** — the highest-consequence window.
- `venues/zerox.rs` `query_order` returns a **hardcoded `Filled`** with dummy
  values, telling the reconciler a DEX trade settled when it may not have; and
  its `10^18` token scaling mis-sizes any non-18-decimal token (USDC/USDT).

## Guiding constraints

- **ADR-0008 (live = replay).** Any sizing/indicator/strategy change must run
  through the one shared codepath so live and backtest stay byte-identical.
  Sizing cannot ship live-only.
- **ADR-0007 / ADR-0010 (one frozen canonical strategy format, three front
  doors).** AND/OR conditions and exit nodes require an **additive
  definition-version bump** (v1.5), following the precedent already set by the
  v1.5 universe nodes in `NodeKind`. SMA/ATR and `PercentOfBalance` do **not**.
- **Security gate.** The non-loopback bind guard (`apps/platform/src/main.rs`)
  is the single line protecting the placeholder auth. It is removed **last**,
  only after end-to-end session auth is verified and TLS/CSRF are in place.
- **Fail-honest over fail-silent.** Where a real implementation is deferred,
  return a clear error (`NotImplemented`, 503) — never fabricate success.

---

## Phase Summary

| Phase | File | Label | Tasks | Completion | Goal |
|-------|------|-------|-------|------------|------|
| 0 | [phase-0.md](phase-0.md) | Correctness & safety quick wins | 5 | **100% (5/5)** | fix the live-trading hazards that are cheap to fix now |
| 1 | [phase-1.md](phase-1.md) | Authentication & multi-user | 7 | **86% (6/7) — 1.7 awaits human security review** (2026-10-03) | real session auth; unlock network binding. **4 of 7 block unattended paper** |
| 2 | [phase-2.md](phase-2.md) | Live P&L, manual orders & shared data | 3 | **100% (3/3)** (2026-10-03) | real $ rollup + paper manual orders. **2 of 3 block unattended paper** |
| 3 | [phase-3.md](phase-3.md) | Execution venue adapters | 6 | **67% (4/6)** | 3.1/3.2/3.3/3.6 done; 3.5 partial; 3.4 BLOCKED:HUMAN. **None blocks unattended PAPER** |
| 4 | [phase-4.md](phase-4.md) | Strategy surface & sizing | 4 | **25% (1/4)** | 4.1 done; 4.2/4.4 superseded by Set U (v3 engine); 4.3 open |
| 5 | [phase-5.md](phase-5.md) | Data pipeline & observability | 5 | **60% (3/5) — 2 PARTIAL** (2026-10-03) | 5.5 shipped, never ticked. **5.1 and 5.4 are the worst blockers in the set** |
| FS | [future-scope.md](future-scope.md) | Future scope (not counted) | — | — | TigerGraph / Milvus — keep code, gate infra |

**Recommended sequencing:** 0 → 1 → 2 makes the platform safe-to-run and
network-deployable with real P&L. 3 → 4 → 5 broadens venues, the strategy
surface, and operability. Phase 0 can land immediately and independently.

---

## Item → Phase Map

Every finding from the sweep and its home. `CL` = row in `/COMPLACENCY_LOG.md`;
`NF` = new finding from the 2026-06-13 sweep.

| # | Source | Item | Phase · Task |
|---|--------|------|--------------|
| 1 | CL | Placeholder bearer auth accepts any token (M-17, CRITICAL) | 1.3 |
| 2 | CL | Platform loopback-bind guardrail | 1.7 |
| 3 | NF | `/auth/*` routes don't exist; frontend auth UX is dead | 1.2 |
| 4 | CL | `storage/postgres/users.rs` placeholder (count only) | 1.1 |
| 5 | CL+NF | Automations scoped to `DEV_USER` nil; `streams.rs` raw-token id | 1.4 |
| 6 | NF | Frontend `dev-local` hardcoded bearer token | 1.5 |
| 7 | CL | Coinbase live broker stub (HIGH) | 3.5 |
| 8 | CL | Tradier account adapter `NotImplemented` (HIGH) | 3.1 |
| 9 | CL | Tradovate account adapter `NotImplemented` (HIGH) | 3.2 |
| 10 | CL | 0x `query_order` hardcoded `Filled` + tx-hash stub (MED) | 0.2 / 3.4 |
| 11 | CL | AMM paper sim uses mocked `FirmQuote` (MED) | 3.3 |
| 12 | CL | Manual order placement disabled (503) (HIGH) | 2.3 |
| 13 | CL | Dashboard live P&L rollup all-zero (HIGH) | 2.2 |
| 14 | CL | Prometheus metrics no-op stubs (MED) | 5.1 |
| 15 | CL | Correctness metrics no-op stubs (MED) | 5.1 |
| 16 | CL | `OrderBookBuilder` empty struct (MED) | 5.3 |
| 17 | CL | Freshness UTC-only hours check (MED — promoted to safety) | 0.1 |
| 18 | CL | Parquet compaction no-op (LOW) | 5.2 |
| 19 | CL | MCP discovery static fabricated data (LOW) | 0.3 |
| 20 | CL | WS serialize silent failure `unwrap_or_default` (LOW) | 0.4 |
| 21 | CL | MCP server has zero tests (MED) | 5.5 |
| 22 | NF | `PercentOfBalance` / `RiskUnit` sizing parse-only | 4.2 / 4.5 |
| 23 | NF | Builder SMA/ATR indicators unsupported | 4.1 |
| 24 | NF | Builder AND/OR conditions + exit rules dropped (format bump) | 4.4 |
| 25 | NF | `ScannerPanel` strategy-change stub (no tile population) | 4.3 |
| 26 | NF | Hot-path stage-3 strategy instance is a `None` placeholder | 5.4 |
| 27 | NF | Milvus port discrepancy (19530 vs 9091) + always-on infra | 0.5 / FS |
| 28 | NF | TigerGraph unused, always-on infra | FS |
| 29 | CL | Config secrets (`config/default.toml`) + ops docs (LOW) | future-scope §3 |

---

## Locked decisions (2026-06-13)

All 12 design decisions were resolved by the product owner. The consistent
directive was **take the most robust option, no shortcuts**.

| # | Phase | Decision | Locked choice |
|---|-------|----------|---------------|
| 1 | 1 | Auth mechanism | **Opaque server-side cookie sessions** (`HttpOnly`+`SameSite`, session table). Frontend change = delete the dev-token interceptor; add CSRF protection. |
| 2 | 1 | Tenancy | **Multi-tenant with hard cross-user isolation.** Every resource filtered by owner; isolation is a security requirement. |
| 3 | 1 | Legacy `Uuid::nil()` data | **Delete legacy rows.** Wipe nil-user automations/backtests on the auth cutover (disarm any armed automation first as a safety step). |
| 4 | 2 | Mark source / staleness | **Trade-last from Redis, surface staleness.** Latest trade-lane price is the mark; missing/expired marks are surfaced to the user, never silently zeroed. |
| 5 | 2 | Manual orders first cut | **Paper-only.** Wire paper-mode now; gate live behind "broker not available." |
| 6 | 3 | Tradovate credentials | **`username:password` in-adapter exchange.** Adapter calls `/auth/accesstoken`, handles token renewal. |
| 7 | 3 | 0x execution model | **Full on-chain submit + poll.** Adapter signs + broadcasts the swap and polls `eth_getTransactionReceipt` for status; needs a signer wallet + RPC + secure key management. Task **3.4 promoted into scope**. |
| 8 | 3 | Coinbase live | **In scope — build live.** Full Coinbase Advanced Trade `Broker` with ES256 JWT per-request signing (`jsonwebtoken`). Real-money path. Task **3.5 promoted into scope**. |
| 9 | 4 | Strategy format | **Approve additive v1.5 bump** for AND/OR + exit nodes; v1.0 strategies keep working. |
| 10 | 4 | Equity basis (PoB) | **Total account equity** (cash + open-position value), identical figure live and in the SDK. |
| 11 | 4 | Universe feed | **One shared feed** for ScannerPanel discovery and future Automations pipelines. |
| 12 | 5 | Hot-path hand-off | **Real `PipelineFactory` in the demand-manager** (replaces `NoopPipelineFactory`, owns pipeline+instance lifecycle), using an arc-swap snapshot internally for the lock-free stage-3 read. |

Phase files reflect these choices. Safety notes: (3) armed automations are
disarmed before deletion; (7,8) the 0x on-chain adapter and the Coinbase live
broker are **real-money paths** — keys/secrets live in secure storage (never
config/repo), and each is gated by a `/security-review` of key handling before
live/mainnet enablement.

---

## Progress Log

Update this table and the per-phase completion headers as tasks land.

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-13 | — | plan | Set E created from the repo-wide stub/stale sweep + 5-cluster deep research; consolidates COMPLACENCY_LOG v2 (21 items) + 8 new findings. **Deferred-by-design:** 4.5 (RiskUnit — blocked on exit nodes), plus the future-scope appendix (TigerGraph, Milvus). |
| 2026-06-13 | — | decisions | All 12 design decisions locked. **3.4 (0x full on-chain submit/poll) and 3.5 (Coinbase live) promoted from deferred into scope** per decisions 7 & 8 — both real-money paths, gated by a key-handling security review. Decision 12 set to a real `PipelineFactory` in the demand-manager (arc-swap internally). |
| 2026-09-15 | 2,4,5 | 2.3, 4.2, 5.4 (+4.4 partial) | **Track 16 — the execution path to a fill.** An audit found there was no way to get a fill out of the platform at all: hot-path stage 3 was `Option::None`, `armed_automations` had no caller, `PercentOfBalance` returned `None`, exits were dropped on save, and nothing anywhere wrote an order or a fill. **5.4** wires stage 3 to one live-strategy registry (**ADR-0038**: automations declare, the hot path evaluates — no second evaluator) and replaces stage 4's eight-constant `GateContext`. **2.3** and stage 4 now share one `RiskContextProvider`; the rate counters live inside the gate, so the rate and daily-loss limits can fire at all for the first time. **4.2** PoB sizes live (replay call site is track 15's `sim.rs`). **4.4** partial: exits are now *refused on save* instead of silently dropped; the v1.5 bump is still open. Migration **0121** makes orders and fills durable and idempotent at the database (SC-7). Proven by 11 tests in `apps/platform/src/hot_path.rs`. |
| 2026-06-13 | 0 | all 5 tasks | **Phase 0 complete.** 0.1: freshness.rs now converts UTC→venue local time via `chrono-tz` before comparing to session windows; 6 new timezone-correct tests. 0.2: zerox.rs `query_order` returns `Unknown` (not `Filled`); token scaling uses per-symbol decimal map (USDC/USDT=6). 0.3: `list_lanes`/`list_instruments` use `domain::lanes::ALL_LANES` and `domain::instrument::ALL_ASSET_CLASSES`; `list_instruments` now queries the live Postgres `instruments` table (async MCP server, `DATABASE_URL`-driven). 0.4: `json_msg` returns `Option<Message>`; serialize failures are logged and frame is dropped. 0.5: TigerGraph/Milvus/etcd/minio behind `profiles: ["vector-graph"]`; embedder `MILVUS_HTTP_PORT` default corrected to 19530. |
| 2026-10-03 | 1,2,5 + audit | 1.1–1.7, 2.1, 2.2, 5.1–5.3, #3–#5, #8, #9 | **Track PROD.** Sessions store SHA-256 digests (0150, DB-side hashing trigger), sliding web expiry, lazy sweep; `/auth/touch` + venue credentials built (mount pending); WS verified on the shared resolver; automations owner-scoped and nil rows deleted (0151); `dev-local` verified gone; network-bind predicate built; live rollup from the caller's ledger with surfaced mark coverage (0154); metrics registry + exporter; crash-safe parquet compaction; L2 `OrderBookBuilder`; append-only order audit (0152); durable paper accounts (0153); poison-tolerant paper engine; Alpaca inbound side/type refused when unknown. Shared-file edits (routes, `main.rs`, three cleartext-token readers) listed in the track report. |
| 2026-09-15 | all | audit | **Re-verified all 30 items against the code** (plan-hygiene track, ROADMAP Stage 3). True count **6 / 30** (was 5): 6 DONE, 7 PARTIAL, 17 OPEN, 0 obsolete. 5.5 had shipped and was never ticked; 1.3 was built without being claimed. Phases 3, 4 and 5.1–5.4 untouched since the sweep. **Ten items marked as blocking unattended paper trading**, plus ten untracked code defects recorded in the audit header — five of which (no order/fill persistence, no durable idempotency, no order audit trail, a kill switch that does not survive restart, a non-durable paper engine) are prerequisites Stage 3's exit criteria assume already exist. |
