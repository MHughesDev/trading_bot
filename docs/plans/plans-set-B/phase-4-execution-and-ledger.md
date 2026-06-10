---
Type: Formal
Status: Pending
Derived From: C-017, C-056, C-058, C-059, C-068, C-073, C-086, C-087, C-088, C-092, C-105
---

# Phase 4 — Execution & Ledger

> **Self-contained execution doc.** You need only: this file, [`../../architecture.md`](../../architecture.md),
> the specs (especially
> [`../../specs/COMP-002-execution-and-risk-gate.md`](../../specs/COMP-002-execution-and-risk-gate.md)),
> and the existing codebase. Phase 1 built the paper-simulator trait + skeletons, ledger tables +
> writer skeleton, `AccountSource` trait, and P&L lot schema — this phase completes them. Phase 2
> built the venue collectors + health checks. Read `crates/execution` (broker trait, order_state,
> fills, positions) and `crates/storage` (ledger, pnl) before editing.

## Phase goal

After this phase the **money plane is complete end-to-end**: all four internal paper fill simulators
(+ DEX wallet) are fully implemented; the new venue broker adapters (OANDA, Kalshi, Tradier, 0x,
Tradovate-demo) exist behind the broker trait; the execution router decides **`PAPER` vs
`LIVE_ROUTED`** per order and dispatches accordingly; per-venue `AccountSource` REST adapters fetch
balances/positions/transactions on demand; the event-sourced **ledger writer** records every
fill/fee/margin/funding event; and the **USD P&L rollup engine** produces the three-tier dashboard
numbers (platform → asset class → venue) with FIFO lot matching.

## Prerequisites

- Phase 1 done: paper simulator trait + 4 skeletons + DEX wallet, ledger tables (`0007`) + writer
  skeleton, `AccountSource` trait, P&L lot schema (`0008`), credential service.
- Phase 2 done: venue collectors + per-venue health checks + `SupportedVenue` capabilities.

## Invariants this phase must respect

- **Execution modes are `PAPER` or `LIVE_ROUTED` only** (C-058). The router has exactly these two
  branches; there is no Alpaca-paper hybrid.
- **Internal paper only** (C-056): the `PAPER` branch calls a paper simulator, never a venue.
- **No `f64` on price/size**; ledger/lot/P&L math uses `Decimal`-backed types.
- **Append-only ledger** (C-088): the writer is insert-only; corrections are new events.
- **Idempotency on money paths**: fills key by fill id; ledger appends key by source event so a
  redelivery is a no-op.
- **No risk UI / no backtest** (C-114): internal validity checks may reject malformed orders but are
  not labeled "risk"; there is no backtest path.

## Decision gate (resolved)
- **Q-exec-mode:** Two modes only — `PAPER` (internal simulator) and `LIVE_ROUTED` (venue adapter)
  (C-058/C-059). Live execution venues per the vision: Coinbase (crypto), Alpaca (equity), OANDA (FX
  demo), Kalshi (prediction/perps), Tradier (options), 0x (DEX), Tradovate (futures demo).

---

## Tasks

### P4-T01 — Complete the four paper fill simulators + DEX wallet
- **Goal:** Full, deterministic implementations of all four simulators and the DEX paper wallet.
- **Files:** `crates/execution/src/paper/{clob,broker_quote,amm_swap,prediction,wallet}.rs`.
- **Context:** Per C-056/C-086/C-087. Complete behaviors beyond the Phase 1 skeletons:
  - `CLOBFillSimulator` (crypto/futures/perps): market + limit + bracket support, partial fills,
    fee model, slippage in ticks against the 1-min mark.
  - `BrokerQuoteFillSimulator` (equities/options/FX): synthetic NBBO from mark ± half-spread, TIF
    handling, marketable-limit logic.
  - `AMMQuoteSwapSimulator` (DEX): fills against a real 0x `FirmQuote` (from the Phase 2 `DexQuote`
    feed), applies the quoted price + gas, debits/credits the `DexPaperWallet`.
  - `PredictionMarketFillSimulator` (Kalshi): YES/NO binary fill in [0,1] with contract sizing.
  - `DexPaperWallet`: per-token simulated balances, rejects swaps exceeding balance.
  Every fill produces a `Fill` carrying fee + the data needed for a ledger row.
- **Acceptance:** `crates/execution/tests/paper_simulators.rs` proves each simulator fills its market
  structure correctly (CLOB partial fill aggregates; broker-quote respects TIF; AMM debits the wallet
  and rejects insufficient balance; prediction fills in [0,1]) — all deterministic, no network.
- **Depends on:** Phase 1 paper skeletons, Phase 2 `DexQuote` feed.

### P4-T02 — Execution router: PAPER vs LIVE_ROUTED
- **Goal:** A single router that, per order, dispatches to a paper simulator or a live venue adapter
  based on the account mode.
- **Files:** `crates/venue-router/src/exec_router.rs` (new), `crates/execution/src/lib.rs` (expose a
  unified `submit`).
- **Context:** Per C-058/C-059. `ExecutionMode { Paper, LiveRouted }`. The router resolves
  `(AssetClass, SupportedVenue)` → either the market-structure paper simulator (via
  `AssetClass::market_structure()`) for `Paper`, or the venue broker adapter for `LiveRouted`. The
  runtime and UI never know which fired. Internal validity checks (malformed quantity, unsupported
  order type) may reject here but are **not** called "risk." Paper and Live are separate account
  levels — a `Paper` order never touches a venue.
- **Acceptance:** `crates/venue-router/tests/exec_router.rs` proves a `Paper` crypto order routes to
  `CLOBFillSimulator` (no network) and a `LiveRouted` equity order routes to the Alpaca adapter; an
  unsupported order type is rejected with a typed (non-"risk") error.
- **Depends on:** P4-T01, P4-T03.

### P4-T03 — New venue broker adapters
- **Goal:** Broker-trait adapters for OANDA, Kalshi, Tradier, 0x, Tradovate (demo first).
- **Files:** `crates/execution/src/venues/{oanda,kalshi,tradier,zerox,tradovate}.rs` (new), register in
  `crates/execution/src/lib.rs`. (Coinbase live and Alpaca live already exist/are stubbed.)
- **Context:** Per the vision live-execution mapping. Each implements the existing `Broker` trait
  (`submit`, `cancel`, `query_open_orders`, `query_positions`) against its venue's live (or demo) API:
  OANDA v20 (FX demo), Kalshi (prediction/perps), Tradier (options), 0x (DEX swap execution via firm
  quote), Tradovate (futures demo). All money fields use `Price`/`Size`. On a missing ack, **query,
  never blind-retry** (carry the idempotency key).
- **Acceptance:** `crates/execution/tests/venue_adapters.rs` proves each adapter constructs a correct
  venue request from an `OrderIntent` and parses a sample venue ack/fill into the internal fill type
  (mocked HTTP; no live calls in CI).
- **Depends on:** Phase 2 (`SupportedVenue` capabilities, health checks).

### P4-T04 — Ledger writer: record every money event
- **Goal:** Wire the execution path to append a ledger event for every fill, fee, margin event, and
  funding payment.
- **Files:** `crates/storage/src/ledger.rs` (complete), `crates/execution/src/events.rs` (emit ledger
  appends), `crates/execution/src/positions.rs` (margin/funding hooks).
- **Context:** Per C-088/C-068. On each fill, append a `Fill` ledger event with the typed asset payload
  + context (venue, strategy, before/after position, USD conversion). Fees append a `Fee` event.
  Perp/futures funding append `FundingPayment`; margin changes append `Margin`. **Idempotent**: a
  redelivered fill (same fill id) appends nothing new. Insert-only.
- **Acceptance:** `crates/storage/tests/ledger_writer.rs` proves a fill produces exactly one `Fill`
  ledger row, a redelivered fill produces none, and a funding event produces a `FundingPayment` row
  with the correct typed payload; `seq` is monotonic.
- **Depends on:** P4-T01, Phase 1 ledger schema.

### P4-T05 — P&L lot engine (FIFO) + realized/unrealized
- **Goal:** Maintain event-sourced FIFO lots from ledger fills; compute realized P&L on close and
  unrealized at current mark, all in USD.
- **Files:** `crates/storage/src/pnl.rs` (new or complete), `crates/execution/src/positions.rs`
  (drive lot updates from fills).
- **Context:** Per C-073/C-105. An opening fill creates a `pnl_lots` row; a closing fill consumes the
  oldest open lots first (FIFO), writes `pnl_closes` rows with `realized_usd`. Unrealized P&L =
  `remaining_qty × (current_mark − open_price)` converted to USD at the current rate. Win rate =
  % of **closed positions** that are profitable (position-level, not per-lot). All conversions use the
  USD baseline.
- **Acceptance:** `crates/storage/tests/pnl_engine.rs` proves: two opens + one partial close realize
  USD P&L against the oldest lot first; remaining unrealized reflects the current mark; a position that
  closes net-positive counts toward win rate and a net-negative one does not.
- **Depends on:** P4-T04.

### P4-T06 — USD rollup engine (three-tier dashboard numbers)
- **Goal:** Aggregate P&L and win rate into platform-wide → per-asset-class → per-venue tiers, in USD,
  for Paper and Live separately.
- **Files:** `crates/api/src/rollup/mod.rs` (new), `crates/api/src/routes/dashboard.rs` (new endpoint).
- **Context:** Per C-073/C-105/C-079/C-080/C-081. `GET /api/dashboard/rollup?mode=PAPER|LIVE` returns:
  top tier (platform total P&L + win rate, USD), middle tier (one entry per of the 8 asset classes
  with its P&L/win rate), bottom tier (per-venue tiles within each class). Computed from `pnl_lots`/
  `pnl_closes` + current marks. **On-demand only** — this is computed when the Dashboard requests it,
  not on a background loop. Paper and Live are always separate account levels.
- **Acceptance:** `crates/api/tests/rollup.rs` proves the rollup sums per-venue into per-class into
  platform total consistently, that Paper and Live are isolated, and that win rate is position-level.
- **Depends on:** P4-T05.

### P4-T07 — Per-venue `AccountSource` REST adapters
- **Goal:** On-demand balance/position/transaction fetch per venue, implementing the Phase 1
  `AccountSource` trait.
- **Files:** `crates/execution/src/account/{coinbase,kraken,alpaca,oanda,kalshi,tradier,tradovate}.rs`
  (new), register in `crates/execution/src/account_source.rs`.
- **Context:** Per C-017/C-092. Each adapter authenticates with the user's stored (decrypted) venue
  credentials and fetches balances, positions, and recent transactions via the venue REST API,
  normalizing to the `AccountSource` DTOs (`Price`/`Size`). **Fires on-demand** when the user
  navigates to Dashboard — no polling. Credentials are decrypted just-in-time and never logged.
- **Acceptance:** `crates/execution/tests/account_adapters.rs` proves each adapter maps a sample venue
  balances/positions response into the DTOs correctly (mocked HTTP), and that a missing/invalid
  credential yields a typed auth error, not a panic.
- **Depends on:** Phase 1 (`AccountSource` trait, credential service), P4-T03.

---

## Phase exit criteria
- [ ] All four paper fill simulators + DEX wallet are fully implemented and deterministic (no network);
      simulator tests green.
- [ ] The execution router dispatches `PAPER` → simulator and `LIVE_ROUTED` → venue adapter, with
      exactly those two modes; a paper order never touches a venue.
- [ ] OANDA, Kalshi, Tradier, 0x, Tradovate broker adapters implement the `Broker` trait; adapter
      tests green (mocked HTTP).
- [ ] The ledger writer appends one event per fill/fee/margin/funding, idempotently and insert-only.
- [ ] The FIFO P&L lot engine computes realized (on close) and unrealized (at mark) P&L in USD with
      position-level win rate.
- [ ] The USD rollup engine returns platform → asset-class → venue tiers for Paper and Live
      separately, on-demand.
- [ ] Per-venue `AccountSource` adapters fetch balances/positions/transactions on demand using
      decrypted credentials.
- [ ] `cargo check --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
      --all-features` all green.
