# Phase 3 — Simulation Fidelity

**Completion: 80% (4 / 5 — 3.1, 3.2, 3.4, 3.5 done; 3.3 `BLOCKED:UPSTREAM` on the market_simulator SDK)**

**Goal:** Bring backtest behavior closer to live, and fix the fidelity-affecting
heuristics. **Addresses:** #4, #5, #6, #7, #9, #14

---

## Tasks

### ☑ 3.1 Replay stored features instead of recomputing — L
**Addresses #4.** The platform records versioned feature values at their
`available_time`; replay now sees exactly those, not recomputed ones.
- `BarStore::load_features` reads `features_technical` keyed by `available_time`
  (deduplicated with `argMax(value, ingested_time)`); the manager loads them in
  the LoadingData phase and hands them to the sim.
- In `build_handler` each feature resolves to its stored value when one exists
  for the bar's timestamp, otherwise the indicator is recomputed.  Fallback
  indicators are advanced on every bar so they stay warm — a run with no stored
  features behaves exactly as before.
- Preserves the platform's versioned-feature invariant and live/replay parity
  (ADR-0008).
- **Files:** `crates/backtest/src/store.rs` (`load_features`, `StoredFeatures`),
  `sim.rs` (resolve path), `manager.rs` (load + wire).
- **Verify:** `sim.rs::stored_features_override_recomputed_indicators` — injected
  stored values flip a decision the recomputed indicators never would, proving
  the stored value drove the order.

### ☑ 3.2 Fix the gap-merge over-reach — S
**Addresses #14.** On continuous markets, two missing days separated by a
*present* day can merge and swallow the present day into the reported range.
- In `gaps::merge_days`, only merge across genuinely non-trading days; don't
  bridge across a present day.
- **Files:** `crates/backtest/src/gaps.rs`.
- **Verify:** test the N+1 / N+3 continuous-market case stays two ranges.

### ☐ 3.3 Tick / quote replay — L (`BLOCKED:UPSTREAM`: needs an out-of-repo SDK change)

> `BLOCKED:UPSTREAM` 2026-10-03 (track LEGACY). Re-verified in code: neither backtest engine
> consumes ticks or quotes. v1 `sim.rs` is bar-only through the pinned SDK; the Set U engine's
> `Event` enum (`crates/strategy-engine/src/engine.rs`) is `Bar | Fill | OrderClosed | Timer`,
> so Set U does **not** cover this. Nothing in this repo can close it. **What the
> `market_simulator` SDK needs** (separate repo, `crates/backtest/src/sdk.rs`):
> 1. A `TickHandler`/`QuoteHandler` registered next to `BarHandler`, taking `TradeTick`
>    (`ts_event`, `price`, `size`, `aggressor_side`, `trade_id`) and `QuoteTick`
>    (`ts_event`, `bid`, `ask`, `bid_size`, `ask_size`).
> 2. A `SimDataFeed::push_trade_tick` / `push_quote_tick` intake that merges ticks and bars into one
>    time-ordered stream (stable by `ts_event`, then by feed kind) so a bar never precedes the
>    ticks it was built from.
> 3. Fill model: a market order fills at the touch of the last `QuoteTick` (not the bar close);
>    a resting limit fills when a `TradeTick` crosses it. Keep `SimOrderCommand` unchanged.
> 4. A new immutable tag (D-0.1/0.2) so this repo bumps the pinned `rev` once.
>
> **This repo's half, after the SDK ships:** `BarStore::load_trades` (from `market_trades`,
> dedup `argMax(.., ingested_time)`), the manager's LoadingData phase, a `requirements.rs`
> coverage rule for the tick lane, and a Set U engine `Event::Trade`/`Event::Quote` variant
> (U-territory, to be requested from the Set U owner). Quote history does not exist in the
> store yet (Set P book/quote lane), so quote replay also waits on Set P.
**Addresses #5.**
- Feed `QuoteTick`/`TradeTick` from `market_trades` when a strategy subscribes to
  those lanes, not just OHLCV bars; route them through the SDK.
- **Files:** `crates/backtest/src/store.rs`, `sim.rs`; SDK needs a quote/trade
  intake helper (market_simulator `sdk.rs`).
- **Why still deferred:** the frozen SDK surface
  (`nautilus_backtest::sdk`, pinned at an immutable `rev`) exposes only a
  `BarHandler` + `SimOrderCommand`; there is no tick/quote intake. Adding one
  means editing `market_simulator/crates/backtest/src/sdk.rs` — a **separate
  repository, outside this repo's push scope** (ADR-0014, Phase 0). Bars-only
  is the deliberate v1.0 contract; tick replay waits on a coordinated SDK
  release in that repo.

### ☑ 3.4 Validate warm-up empirically — M
**Addresses #9.**
- Replace the 5×period / period+1 / floor-30 heuristics with measured indicator
  convergence (or expose them as config) and document the rationale.
- **Files:** `crates/backtest/src/requirements.rs`.

### ☑ 3.5 Broaden & pin down the strategy surface — M (`DONE` 2026-10-03, absorbed by Set U)

> `DONE` 2026-10-03 (track LEGACY). The deferred item was non-`Fixed` sizing, blocked on the
> shared runtime. Both halves have since landed: the shared `strategy-runtime` builds
> `PercentOfBalance` intents from a `SizingContext` (Set E 4.2; `intents.rs`, and the live
> stage 3 sets the context -- `hot_path::percent_of_balance_sizes_a_real_order`), and the Set U
> engine plans `units | notional | equity_fraction | risk | vol_target` sizes
> (`strategy-engine/src/planner.rs::qty`, pinned by `tests/phase1_acceptance.rs` and
> `crates/backtest/tests/portfolio.rs`) for the **same** engine in backtest, paper and live, so
> the parity concern in the note below is met by construction. The legacy v1 `sim.rs` path stays
> fixed-size on purpose: it fails fast with a plain-language message
> (`manager.rs::humanize_error`), and v1 strategies are superseded by v3 authoring. Not
> re-implementing sizing in `sim.rs` is the decision (ADR-D-3.5: rejected alternative was a
> backtest-only `PercentOfBalance` in `sim.rs`, which would diverge from the one engine).
**Addresses #6, #7.**
- ☑ **Rising-edge semantics pinned** against a deterministic EMA-cross case
  (`sim.rs::ema_cross_over_rising_bars_places_one_order`, 4.2) — one crossover ⇒
  exactly one order.
- ☑ **Supported surface documented explicitly** (FEAT-002 §2/§6): v1.0 bars-only,
  `Fixed` sizing, condition/signal nodes. v1.5 universe nodes
  (Rank/Filter/TakeTopN/DataSource/Surface) are **out of scope for backtest** —
  they describe cross-instrument selection, whereas a run is bound to a single
  instrument; they belong to a future portfolio-level backtest, not this one.
- ☐ **`PercentOfBalance` / `RiskUnit` sizing — deferred.**
- **Why sizing is still deferred:** these modes are parse-only in the **shared**
  `strategy-runtime::intents::build_intent_from_action`, which returns `None` for
  them — i.e. they are non-executable in the **live** path too. Implementing them
  in `sim.rs` alone would make replay place orders that live trading would not,
  breaking the live/replay parity invariant (ADR-0008) that the whole bridge is
  built to preserve. Broadened sizing must therefore land in the shared runtime
  (with account-equity plumbed through the intent build) **first**; doing it
  backtest-only would be a fidelity regression, not a gain.
- **Files:** `crates/backtest/src/sim.rs`, docs.

---

## Definition of Done
Indicator values match the live/stored feature pipeline; gap ranges are exact;
warm-up is justified; the supported strategy surface is documented and broadened
where intended; (optionally) tick replay available.
