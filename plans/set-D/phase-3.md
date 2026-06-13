# Phase 3 — Simulation Fidelity

**Completion: 60% (3 / 5 — 3.1, 3.2, 3.4 done; 3.3/3.5 deferred for the reasons below & in FEAT-002 §6)**

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

### ☐ 3.3 Tick / quote replay — L (deferred: needs an out-of-scope SDK change)
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

### ☐ 3.5 Broaden & pin down the strategy surface — M (partially landed)
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
