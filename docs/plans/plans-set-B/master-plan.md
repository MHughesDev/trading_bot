---
Type: Formal
Status: Current
Derived From: SYS-001, C-012, C-013, C-015, C-017, C-026, C-053, C-055, C-056, C-058, C-059, C-060, C-061, C-065, C-068, C-072, C-073, C-077, C-079, C-080, C-081, C-082, C-084, C-085, C-086, C-087, C-088, C-089, C-090, C-091, C-092, C-093, C-094, C-095, C-096, C-098, C-099, C-100, C-101, C-102, C-103, C-105, C-106, C-110, C-112, C-113, C-114, C-117, C-118, C-119, C-120, C-123, C-126, C-127, C-129, all ADRs
---

# Master Plan — Set B: The Full End-State Trading Platform

> **This is the top-level plan for the finished system.** It defines the vision, the rules every
> phase obeys, the phase sequence, and the entry point. Read this file first, then execute one phase
> file at a time, in order. Every phase file is **self-contained** — it can be executed by Claude
> Haiku 4.5 with only that file plus the existing codebase, no memory of this conversation.
>
> **➤ About to start implementing? Jump to [§6 START HERE](#6-start-here--execution-entry-point).**
> The first file you execute is [`phase-1-registry-and-paper-engine.md`](./phase-1-registry-and-paper-engine.md).

---

## 1. The vision — what the finished system is and feels like

A professional multi-asset trading platform: a **single Rust monolith** (the `platform` binary) with
**satellite collector binaries** (one per venue), fronted by a **React + Vite SPA**. A pro trader
opens it and immediately feels at home.

Five top-level sections reached via an **animated glass-pill navigation button** (Framer Motion,
380 ms asymmetric easing, `prefers-reduced-motion` honored): **Dashboard, Trading, Automations,
Strategy Creation, Settings**.

- **Dashboard** — three-tier account rollup. Top tier: platform-wide P&L + win rate (USD baseline).
  Middle tier: a horizontal infinite slider of per-asset-class vertical slices (Crypto Spot, Equities,
  FX, Prediction Markets, Options, DEX/AMM, Perpetuals, Futures). Bottom tier: per-venue tiles inside
  each slice. Loads on-demand when the user navigates to Dashboard (no background polling). Paper and
  Live are always separate account levels.
- **Trading** — full-height horizontal infinite scroll of panels. Each panel is a **scanner/watchlist**
  (tile-based watch table, no visible column dividers, rounded tiles) or a **terminal** (1-min OHLCV
  chart with order/stop/take-profit annotations drawn on the candles, an asset-class-specific order
  ticket, positions, working orders, fills, account context). **No order-book tables, no DOM/depth,
  no recent-trades tick lists.** Minimum data baseline is 1-minute OHLCV; everything else is derived.
  A Paper/Live toggle (top-right) applies to all panels in that browser window. Multi-display: two
  full browser windows, same account, independently positioned on the same horizontal scroll.
- **Automations** — its own URL. Two flows: **Single Instrument** (asset class + instrument + one
  execution strategy + time window) and **Pipeline** (asset class + universe, ordered filter stages
  as columns, one final execution action). The pipeline board shows live stage-membership counts,
  enter/exit deltas, and instruments flowing in/out continuously. Time-window selector is
  asset-class-aware (24/7 vs sessioned). Armed automations route orders through the strategy runtime
  and execution router — no per-order confirmation.
- **Strategy Creation** — a node-graph builder. **Discovery** strategies (no execution block →
  populate scanner panels) vs **execution** strategies (has execution block → run in Automations).
  Strategy kind is **inferred, never declared**. On save, data requirements compile into a capability
  manifest. The apply list shows only strategies compatible with the selected asset/venue/universe
  (incompatible are hidden, not flagged). Default strategy: 7-period EMA over 21-period EMA, 1-min
  OHLCV only, cross-asset compatible.
- **Settings** — Profile, Venue Credentials (verify-before-save, AES-256-GCM envelope encryption),
  Data & Privacy, Notifications, Appearance, Keyboard Shortcuts. **No risk settings** — risk is not a
  user-facing concept.

**Backend shape:** 8 asset classes (`CryptoSpotCex`, `Equity`, `Fx`, `PredictionMarket`, `Option`,
`CryptoSpotDex`, `PerpetualSwap`, `FuturesExpiring`). Venues: Kraken (crypto data), Coinbase Advanced
Trade (crypto live), Alpaca (equity data + exec), OANDA (FX, demo MVP), Kalshi (prediction +
perpetuals), Tradier (options), 0x (DEX swap aggregation), Tradovate (futures, demo first). Two
execution modes only: `PAPER` (internal fill simulator) and `LIVE_ROUTED` (venue adapter). Live data
flows collectors → NATS JetStream → browser via NATS.ws directly. An event-sourced ledger records
every fill/fee/margin/funding event. TigerGraph stores the capability/compatibility graph; Milvus
stores semantic text (Reddit, web, strategy descriptions).

---

## 2. What already exists (do NOT recreate)

The Rust workspace is substantially built and **all Rust CI passes** (`cargo check --workspace`,
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features`). Already complete
or nearly so:

- `crates/domain` — `Price`/`Size` newtypes (no `From<f64>`), `EventEnvelope`, 4-timestamp model,
  `TrustTier`, payloads (bar/trade/quote/orderbook), `StrategyDefinition` v1.0 (frozen),
  `Instrument`/`AssetClass`, `OrderIntent`/`OrderState`, `Timestamps`, `TradingSchedule`, `HaltPolicy`,
  `Lane` enum.
- `crates/config`, `crates/observability` — complete.
- `crates/event-bus` — NATS JetStream producer/consumer wrappers.
- `crates/builders` — bar builder with watermark/revision logic.
- `crates/features` — EMA, RSI, rolling window.
- `crates/strategy-runtime` — `WorldState`/`WorldContext`, interpreter, instance lifecycle (partial
  node graph).
- `crates/strategy-validator` — strategy JSON validation.
- `crates/collectors` — Kraken crypto + Alpaca equity data collectors (satellite-ready).
- `crates/storage` — bars, trades, features, instruments, orders, partition logic, Redis, ClickHouse
  writers. Migrations `0001`–`0005` applied.
- `crates/risk` — risk gate, kill switch, limits, overrides. **Per C-114 risk is removed from the
  user-facing product; the kill switch machinery stays as an internal operator tool.**
- `crates/execution` — Alpaca paper adapter (to be **retired**, C-056), Coinbase live stub, fills,
  positions, order-state, mock broker.
- `crates/reconciliation`, `crates/api`, `crates/ui-gateway`, `crates/demand-manager`,
  `crates/venue-router`, `crates/mcp-server` — present (stubs/partial).
- `apps/platform`, `apps/collector-crypto`, `apps/collector-equity`, `apps/mcp-server` — compile.
- `frontend/` — React + Vite SPA with strategy-builder nodes, chart panel, positions panel,
  serialize utilities (pre-existing lint errors intentionally deferred).
- `docs/` — ADRs 0001–0011, specs (COMP/DATA/FEAT/INTG/SYS), architecture.md, plans-set-A (reference
  format).

**Executors must read the relevant crate before adding to it** and extend rather than rewrite.

---

## 3. The one principle

> **Take in the minimum source data, derive everything possible from it, and register that derived
> capability immediately** (C-112).

The minimum source data is the **1-minute OHLCV bar** (plus firm quotes where a market structure
requires them). Order books, DOMs, and tick lists are *not* ingested. Charts, indicators, scanners,
and P&L are all derived. Whenever a new data primitive or derived capability is introduced, it is
registered in the authoritative `DataType` registry the moment it exists, so strategy manifests and
collector capabilities stay in sync automatically.

---

## 4. Hard invariants (non-negotiable — restated in every phase file)

1. **No `f64` on price or size, ever.** Use `domain::money::Price` / `Size` (newtypes over `Decimal`,
   no `From<f64>`). Do not add a `From<f64>` to bypass the compiler.
2. **No backtesting.** There is no backtest engine, no backtest API, no `market_simulator` adapter.
   Replay invariants (`available_time` ordering, pure builders/features) are retained **only** for
   live correctness.
3. **No risk UI.** Risk is not a user-facing concept (C-114). No risk settings, no risk overrides
   surfaced to the user. The kill switch remains an internal operator tool only. Internal order
   validity checks (malformed quantity, unsupported order type) may still reject orders but are never
   labeled "risk" in the product.
4. **Internal paper only.** Paper execution uses internal fill simulators — never an external venue
   call. The Alpaca paper account is removed (C-056). Paper and Live are always separate account
   levels.
5. **Credentials verify-before-save.** Venue credentials are validated against a live venue health
   check before persistence, stored with AES-256-GCM envelope encryption, never logged, never
   returned through the API.
6. **Collectors are server-wide shared.** Collectors are ref-counted by subscription lane key; the
   free-tier rate-limit budget is a server-wide resource; a 120-second warm period precedes teardown
   on zero demand.
7. **Execution modes are `PAPER` or `LIVE_ROUTED` only** — no third mode, no Alpaca-paper hybrid.
8. **Append-only ledger.** Every fill/fee/margin/funding event creates an append-only ledger row with
   a typed asset-specific payload and full context. History is never rewritten.
9. **Minimum source data baseline.** No order-book/DOM/tick-list ingestion or UI. 1-minute OHLCV is
   the floor; everything else is derived (C-112, C-129).
10. **Every decided mechanism gets a test that proves it fires.** Paper fill correctness, envelope
    encryption round-trip, idempotent ledger writes, rising-edge automation execution, apply-list
    filtering, kind inference. "Decided" ≠ "done" until its acceptance test is green.

---

## 5. Phase sequence

| Phase | File | Theme | Gate to start |
|-------|------|-------|----------------|
| **1** | [`phase-1-registry-and-paper-engine.md`](./phase-1-registry-and-paper-engine.md) | Backend foundation: DataType + SupportedVenue + 8-class AssetClass enums, venue_credentials encryption, internal paper fill simulators (4 types) + DEX wallet, ledger event tables, AccountSource skeleton, P&L lot schema | none (existing codebase) |
| **2** | [`phase-2-collector-infrastructure.md`](./phase-2-collector-infrastructure.md) | Shared-collector ref-counting, NATS.ws browser subjects, freshness watchdog, venue health checks, new venue collectors (OANDA/Kalshi/Tradier/0x/Tradovate), Reddit collector, TigerGraph + Milvus infra | Phase 1 done |
| **3** | [`phase-3-strategy-system.md`](./phase-3-strategy-system.md) | Capability manifest, pipeline automation runtime, kind inference, v1.5 builder nodes, apply-list filtering, default EMA seed | Phase 1 done |
| **4** | [`phase-4-execution-and-ledger.md`](./phase-4-execution-and-ledger.md) | Full paper simulators, new venue broker adapters, PAPER/LIVE_ROUTED routing, AccountSource adapters, ledger writer, P&L USD rollup engine | Phases 1 & 2 done |
| **5** | [`phase-5-frontend-trading-workspace.md`](./phase-5-frontend-trading-workspace.md) | Glass-pill nav + 5-section routing, horizontal Trading workspace, scanner + terminal panels, NATS.ws hook, Paper/Live per-window, LayoutTemplateRegistry | Phase 2 done (NATS.ws subjects), Phase 3 helps |
| **6** | [`phase-6-frontend-dashboard-and-automations.md`](./phase-6-frontend-dashboard-and-automations.md) | Dashboard tiered rollup UI, Automations single + pipeline flows, Settings venue credentials | Phases 4 & 5 done |
| **7** | [`phase-7-graph-social-and-polish.md`](./phase-7-graph-social-and-polish.md) | TigerGraph schema + population, Milvus embedding pipeline, web scraper satellite, NEWCOMERS.md, frontend lint, docs finalize | Phases 1–6 done |

**Allowed parallelism:** Phase 3 (strategy system) may begin once Phase 1 is done, in parallel with
Phase 2. Phase 4 needs both 1 and 2. Phase 5 needs Phase 2's NATS.ws subjects. Everything else is
strictly ordered.

---

## 6. START HERE — execution entry point

**To begin, do exactly this:**

1. **Read this master plan** (you are here), then read the phase file you are about to execute.
2. **Execute phases in order:** 1 → 2 → 3 → 4 → 5 → 6 → 7 (with the §5 parallelism allowance). Do not
   start a phase until the previous phase's **exit criteria** are all green.
3. **Within a phase, follow each task's `Depends on:`** order. A task is done only when its
   **acceptance criterion passes** — including the named test where one is required. "It compiles" is
   not "done."
4. **Read before you write.** Open the crate/file the task names before editing; extend existing code,
   do not rewrite working code.
5. **Track progress** by task IDs (`P1-T01`, `P2-T03`, …) in a PR/issue checklist.

**Reference paths each phase cites:**
- Architecture / structural contract: [`../../architecture.md`](../../architecture.md).
- Specs: [`../../specs/`](../../specs/) (e.g. `../../specs/DATA-004-strategy-definition-format.md`).
- ADRs: [`../../adr/`](../../adr/).
- The set-A plans ([`../plans-set-A/`](../plans-set-A/)) are the **format reference** and document the
  already-built foundation; set-B is the delta to the full end-state.

---

## 7. Definition of done for all of set-B

- All 8 asset classes, all listed venues (data + execution adapters) exist; `cargo check --workspace`,
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features` are green.
- Internal paper fill simulators (4 types + DEX wallet) replace the Alpaca paper adapter; execution
  routes only `PAPER` or `LIVE_ROUTED`.
- The event-sourced ledger records every money event; the USD P&L rollup engine produces the
  three-tier dashboard numbers (platform → asset class → venue) with FIFO lot matching.
- Venue credentials are verify-before-save and AES-256-GCM envelope-encrypted.
- Collectors are server-wide shared (ref-counted, 120 s warm period); the browser subscribes to
  OHLCV via NATS.ws directly; the freshness watchdog respects instrument trading hours.
- Strategy capability manifests compile at save; kind is inferred; the apply list filters by
  computed compatibility; the default 7/21 EMA strategy seeds at account creation.
- The pipeline automation runtime maintains stateful stage membership with rising-edge idempotent
  execution.
- The frontend ships all five sections: glass-pill nav, horizontal Trading workspace (scanner +
  terminal, no order book), tiered Dashboard (on-demand), Automations (single + pipeline), Settings
  with venue credentials. Per-window Paper/Live mode persists in localStorage, defaulting to Paper.
- TigerGraph holds the capability graph (rebuildable from Postgres/code); Milvus holds embeddings for
  Reddit, web, and strategy text. The web scraper and Reddit collectors run as satellites.
- `NEWCOMERS.md` reflects the end-state; specs for completed phases advance from `Draft` to
  `Implemented`.
