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

### F-4 Single hard-wired paper broker; live path unreachable **[PAPER HALF FIXED]**
`apps/platform/src/main.rs` built
`ExecutionEngine::new(paper_engine.broker(AssetClass::CryptoSpotCex))` — a
single broker for one asset class, so every order hit the crypto paper
account regardless of class.
**Fix:** the paper half is now structured as its own complete execution
side. Each data pipeline registers its instrument's asset class with the
engine (`register_instrument`), and the new `MultiAssetPaperBroker`
(`paper/dispatch.rs`) resolves the class **per order** and routes it to the
correct internal account — same collector data feeding the mark board for
both halves, execution split at the broker boundary.
**Still open (live half):** there is still no way to route a live order;
`venue-router::ExecRouter` (paper vs `LiveRouted`) and the five live venue
adapters remain unreferenced by the platform binary. Wiring live =
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

Implemented **[FIXED]**:

- **Per-asset-class CLOB tuning.** All CLOB classes (crypto, futures, FX,
  perps) previously shared one simulator with equity-ish defaults (1-cent
  tick, 10 bps fee) — an FX fill paid a crypto taker fee and slipped a full
  cent on EUR/USD (~100× too much). `SimulatorSet` now carries per-class
  overrides: FX = 0.2 bps half-spread, pip tick, commission-free;
  futures = 0.25 tick, ~0.5 bps; perps = 0.1 tick, 5 bps taker.
- **Win/loss statistics.** Accounts now count closing trades and winners
  (cash sells, margin reductions, settlements), so the dashboard win rate
  is real rather than hardcoded 0.
- **Size-dependent impact** (`clob.rs`). Market orders pay extra slippage
  scaling linearly with notional against a per-class depth
  (`impact_bps_at_depth × notional / depth_notional`, capped at
  `max_impact_bps`): a $1M crypto order pays +5 bps, a $5k order pays
  ~nothing.
- **Market-hours enforcement** (`session.rs`). DST-aware, dependency-free
  US-Eastern calendars: equities/ETF/options Mon–Fri 09:30–16:00, bonds
  08:00–17:00, futures Globex week with the 17:00–18:00 break, FX
  Sun 17:00 → Fri 17:00; crypto/perps/DEX/NFT/prediction stay 24/7.
  Enforced at submit when `PaperEngineConfig::enforce_sessions` is on
  (on in the platform via `PaperTradingEngine::realistic()`, off in unit
  tests for determinism).
- **Stale-mark guard.** Marks now carry observation timestamps; submits are
  rejected (`StaleMark`) when the latest mark is older than
  `max_mark_age_secs` (5 minutes in the platform config).
- **Prediction-market fee formula.** Kalshi's actual
  `0.07 × C × P × (1 − P)` (rounded up to the cent) replaces the flat 0.7%
  of notional, which overcharged mid-range prices and undercharged extremes.
- **Options quote realism.** Per-class broker-quote overrides: options fill
  at ~1% half-spread plus a flat commission instead of equity's 3 bps.
- **Perp funding scheduler.** The platform now charges open perp paper
  positions hourly (1 bp per 8h, pro-rated) through
  `PaperTradingEngine::apply_funding`, mirroring live venue cash flows.

Roadmap (remaining):

1. **Queue position for resting limits.** Resting orders currently fill the
   instant the mark *touches* the limit — optimistic vs reality. Require a
   strict cross (touch-through), and optionally fill proportionally to
   traded volume at the level.
2. **Contract multipliers from instrument metadata.** Futures use
   multiplier 1; ES is $50/point. Source multipliers from instrument
   metadata instead of the per-class policy constant.
3. **AMM realism.** Replace flat `price_impact_bps` with constant-product
   (x·y=k) impact from configured pool depth + gas fee in quote terms
   (firm-quote path already exists for 0x wiring in Phase 4).
4. **Equity microstructure extras.** Short locate/borrow fees (shorts are
   currently impossible in cash accounts — fine for long-only, but margin
   equity accounts would need them), margin interest, dividends/corporate
   actions.
5. **Options expiry.** Auto-exercise ITM at expiry (settlement exists but
   must be scheduled); short options need margin treatment (currently cash
   long-only); spread should further widen with moneyness/DTE.
6. **Funding rate source.** The perp funding scheduler uses a flat default
   rate; derive it from collector data (mark vs index premium) when a perp
   collector lands.
7. **Simulated latency/partial-fill jitter.** Optional deterministic-seeded
   delay and fill fragmentation so strategy code experiences live-like
   asynchrony in paper mode.

### Addendum 2026-06-17 — fee realism re-audit

A follow-up pass checked every class's fee constant against current (2026)
real schedules. Three corrections shipped:

- **Crypto spot taker fee 10 bps → 25 bps.** The CLOB default (`0.001`) matched
  no venue in this build — it was a Binance-style number, well below both the
  paper venue (Alpaca crypto, 25 bps taker) and the Coinbase Advanced live
  target (40 bps+ at entry tiers). Paper P&L understated trading cost 2.5–12×.
  Fixed via an explicit `fee_rate` on the `CryptoSpotCex` CLOB override (the
  global default is deliberately left alone so ETFs don't inherit a commission).
- **ETF phantom commission removed.** `Etf` is CLOB-structured but had no
  override, so it inherited the crypto default's 10 bps commission + penny tick.
  ETFs trade commission-free with tight spreads; added an `Etf` CLOB override
  (0 commission, ~1 bp spread).
- **Bond tuning was dead code.** The "25 bps + $1/bond" tuning lived in
  `broker_quote_overrides`, but `Bond` routed to `Clob`, so it was never
  consulted — bonds filled at the crypto defaults. Bonds are dealer-quote
  markets, so `Bond` now maps to `MarketStructure::BrokerQuote`, activating the
  intended tuning.

All other classes (FX, futures, perps, options, equity, DEX, NFT, Kalshi)
were verified realistic against 2026 schedules and left unchanged.

### Architecture note: paper as its own half

After this round the execution layer splits cleanly:

```
collectors (one feed, shared by both halves)
   │  ticks → marks + instrument→asset-class registry
   ▼
PaperTradingEngine ◄── paper half: MultiAssetPaperBroker
   │                    per-class accounts, tuned simulators,
   │                    session/freshness gates, funding
   └─ (live half, still to wire): venue-router ExecRouter
                        → per-venue broker adapters + credentials
```

Paper never calls a venue; live never touches the internal accounts; both
read the same mark board. The remaining work for full symmetry is the live
half of F-4.

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
