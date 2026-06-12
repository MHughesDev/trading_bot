# Paper Trading System Audit — Logic, Rules, Functionality, Realism

- **Date:** 2026-06-12
- **Scope:** the internal paper-execution surface (`crates/execution/src/paper/`),
  its wiring into the platform (`apps/platform`), the API surface
  (`crates/api`), and the frontend mode/dashboard/automations flows
  (`frontend/src`).
- **Status:** findings marked **[FIXED]** were resolved in the branch this
  audit ships on (`claude/paper-trading-audit-9iyp0m`); the rest form the
  realism/functionality roadmap below.

---

## 1. What exists and is sound

The paper system is genuinely internal — **no external venue calls anywhere
on the paper path** (C-056):

| Layer | File | Assessment |
|---|---|---|
| Fill simulators (CLOB, broker-quote, AMM, prediction) | `paper/clob.rs`, `broker_quote.rs`, `amm_swap.rs`, `prediction.rs` | Correct directional spread/slippage, fee accrual, limit-cross checks, TIF handling (IOC/FOK cancel, GTC/Day rest) |
| Account policies per asset class | `paper/policy.rs` | 11 classes; cash / margin / binary semantics, leverage (10× futures+perps, 30× FX), option multiplier 100, per-class seed cash; table-driven |
| Accounts + ledger | `paper/account.rs`, `ledger.rs` | Decimal-only arithmetic, buying-power enforcement, margin checks that never block risk-reducing closes, append-only journal with `sum(cash_delta) == cash − seed` invariant |
| Engine | `paper/engine.rs` | Mark board fed by hot path, resting limit orders filled on cross, idempotent resubmits, bounded order history, documented lock ordering |
| Broker/account adapters | `paper/broker.rs`, `account_source.rs` | `Broker` and `AccountSource` implementations backed entirely by the engine |
| Settlement/funding | engine + account | Binary resolution at 0/1, futures/option expiry settlement, perp funding payments |

Test coverage of this layer is strong (unit + integration:
`paper_accounts.rs`, `paper_simulators.rs`, `paper_clob.rs`,
`paper_dispatch.rs`, plus inline tests).

## 2. Findings — functionality gaps

Ordered by severity.

### F-1 Dashboard rollup endpoint was a stub **[FIXED]**
`GET /api/dashboard/rollup` ignored all data sources and returned zeros for
both modes, so the Paper/Live toggle changed nothing on screen.
**Fix:** paper mode is now served directly from the `PaperTradingEngine` —
per-asset-class account tiles (cash, equity, margin, positions, realized /
unrealized P&L, win rate) plus bot-wide `account_totals`, with the venue
tile list intentionally empty (paper has no venues; execution is internal).
Live mode keeps the ledger-backed shape (still empty until a live broker
adapter is wired — see F-4).

### F-2 `/api/automations` did not exist on the backend **[FIXED]**
The Automations page (list) and both creation flows posted to
`/api/automations`, but no route was registered; the DB table (migration
0010) and row model existed with no query layer.
**Fix:** added `storage::automation` CRUD, plus
`GET/POST /api/automations`, `POST /api/automations/:id/arm|disarm`,
`DELETE /api/automations/:id`. Specs are validated against the typed
`AutomationSpec` model before persisting. At startup the platform loads
armed automations and logs the paper/live split — armed automations are
server-side state, independent of any UI session, and **paper and live
automations coexist** (`account_mode` column). The frontend now shows both
groups at all times with arm/disarm/delete controls.

### F-3 Frontend ↔ backend API mismatch (partially fixed)
The SPA still calls a legacy Python API surface (`/auth/*`, `/status`,
`/portfolio/*`, `/pnl/*`, `/universe/*`…) proxied to `127.0.0.1:8001`,
which no longer exists; `/api` and `/ws` were not proxied at all, and no
`Authorization` header was sent (every Rust route requires a bearer token).
**Fixed here:** `/api` + `/ws` vite proxies to the Rust platform (`:8080`)
and a placeholder `Bearer dev-local` request header (matches the M-17
placeholder auth, loopback-only).
**Still open:** the Rust backend has no `/auth/*` endpoints, so the login
flow, sidebar status/positions, transactions, and asset pages still target
dead endpoints. Phase-2 session auth (M-17) should land Rust-side
`/auth/*` and the legacy pages should be re-pointed or retired.

### F-4 Single hard-wired paper broker; live path unreachable
`apps/platform/src/main.rs` builds
`ExecutionEngine::new(paper_engine.broker(AssetClass::CryptoSpotCex))` — a
single broker for one asset class. Orders submitted through the hot path
all hit the crypto paper account regardless of asset class, and there is no
way to route a live order even though `venue-router::ExecRouter` (paper vs
`LiveRouted`) and five live venue adapters exist. The `ExecRouter` is not
referenced by the platform binary at all.
**Recommendation:** make the hot path resolve `(account_mode, asset_class)`
per order intent — paper → `paper_engine.broker(asset_class)`, live →
`ExecRouter::route(LiveRouted, …)` with per-user credentials from the
credential store.

### F-5 Strategies/automations never actually evaluate
Hot-path stage 3 (`hot_path.rs`) holds `strategy: Option<StrategyInstance>
= None` — a placeholder. `POST /api/strategies/:id/start` registers an
instance in `InstanceManager`, but nothing connects the manager to the
pipeline, so armed automations cannot place orders yet. This is the main
gap between "automations are persisted and resumed" (done) and
"automations trade" (not wired). Tracked by set-C issues #3/#5/#24.

### F-6 Marks exist only for BTC/USD
Only one Kraken pipeline (BTC/USD) feeds `PaperTradingEngine::on_mark`.
Any paper order on another instrument is rejected with `NoMarkPrice`
(correct behaviour, but it means 10 of 11 asset-class accounts can never
trade until their collectors are spawned). The collectors for equity,
futures, FX, options, Kalshi, and DEX exist as satellite apps but are not
started by the platform.

### F-7 Mode toggle was per-tab with no propagation **[FIXED]**
`useModeStore` persisted to localStorage but never synchronized live —
two open tabs could silently disagree, and there was no way to *intend*
disagreement (paper on one monitor, live on another).
**Fix:** the mode now cascades to every open tab via the `storage` event;
a tab opened with `?mode=paper|live` (or via the new ⧉ button next to the
mode badge) is **pinned** — it keeps its own mode in sessionStorage,
ignores the cascade, never broadcasts, and shows a pin icon. This gives
both requested behaviours: one toggle drives all tabs by default, and
side-by-side PAPER + LIVE windows are explicit and visible.

### F-8 Dashboard hid 3 of 11 paper accounts **[FIXED]**
The asset-class slider listed 8 classes; ETF, Bond, and NFT paper accounts
were silently dropped. All 11 now render.

## 3. Findings — realism assessment per asset class

Implemented now **[FIXED]**:

- **Per-asset-class CLOB tuning.** All CLOB classes (crypto, futures, FX,
  perps) previously shared one simulator with equity-ish defaults (1-cent
  tick, 10 bps fee) — an FX fill paid a crypto taker fee and slipped a full
  cent on EUR/USD (~100× too much). `SimulatorSet` now carries per-class
  overrides: FX = 0.2 bps half-spread, pip tick, commission-free;
  futures = 0.25 tick, ~0.5 bps; perps = 0.1 tick, 5 bps taker.
- **Win/loss statistics.** Accounts now count closing trades and winners
  (cash sells, margin reductions, settlements), so the dashboard win rate
  is real rather than hardcoded 0.

Roadmap (highest realism value first):

1. **Size-dependent impact.** Fills ignore order size (`partial_fill_ratio`
   is static). Add a square-root impact term against a configurable
   per-instrument depth so a 100-BTC market order pays more than a 0.01-BTC
   one.
2. **Market-hours enforcement.** Equities/options/futures fill 24/7. Gate
   `BrokerQuote`/futures fills on a DST-aware session calendar (queue or
   reject outside RTH; the `TimeWindow` model already exists on automations).
3. **Stale-mark guard.** A mark from hours ago still fills orders. Reject
   (or flag) fills when the last mark is older than a per-class threshold.
4. **Queue position for resting limits.** Resting orders currently fill the
   instant the mark *touches* the limit — optimistic vs reality. Require a
   strict cross (touch-through), and optionally fill proportionally to
   traded volume at the level.
5. **Prediction-market fee formula.** Kalshi charges
   `0.07 × C × P × (1 − P)` per contract, not a flat 0.7% of notional;
   the flat rate overcharges mid-range prices and undercharges extremes.
6. **Perp funding scheduler.** `apply_funding` exists but nothing calls it
   periodically; add an 8-hourly task using a configurable or derived rate.
7. **Contract multipliers from instrument metadata.** Futures use
   multiplier 1; ES is $50/point. Source multipliers from instrument
   metadata instead of the per-class policy constant.
8. **AMM realism.** Replace flat `price_impact_bps` with constant-product
   (x·y=k) impact from configured pool depth + gas fee in quote terms
   (firm-quote path already exists for 0x wiring in Phase 4).
9. **Equity microstructure extras.** Short locate/borrow fees (shorts are
   currently impossible in cash accounts — fine for long-only, but margin
   equity accounts would need them), margin interest, dividends/corporate
   actions.
10. **Options realism.** Spread should widen with moneyness/DTE; expiry
    should auto-exercise ITM (settlement exists but must be scheduled);
    short options need margin treatment (currently cash long-only).
11. **Simulated latency/partial-fill jitter.** Optional deterministic-seeded
    delay and fill fragmentation so strategy code experiences live-like
    asynchrony in paper mode.

## 4. Multi-window / multi-tab design (as shipped)

- **Default:** mode is shared app-wide. Toggling the badge in any tab
  updates every open tab (localStorage `storage` event — no server round
  trip, works across windows and monitors).
- **Pinned windows:** the ⧉ button next to the badge opens the current page
  in a new window pinned to the *other* mode (`?mode=live|paper`); pins are
  kept in sessionStorage so they survive reload and in-tab navigation, and
  duplicated tabs inherit the pin. Pinned windows show a pin icon and
  toggle only themselves.
- Any page can be opened in any combination (e.g. LIVE trading on monitor
  1, PAPER automations on monitor 2) since the pin is orthogonal to the
  route.
- Server state is mode-agnostic: automations run in both modes
  concurrently; the toggle only selects which account data the UI displays.

## 5. Verification

- `cargo test --workspace` green (includes new tests:
  per-class CLOB tuning, win/loss counters, paper rollup shape/totals/
  win-rate, non-USD exclusion).
- `npm run build` (tsc + vite) green.
