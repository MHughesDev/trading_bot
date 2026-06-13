# Phase 3 — Simulation Fidelity

**Completion: 0% (0 / 5 tasks complete)**

**Goal:** Bring backtest behavior closer to live, and fix the fidelity-affecting
heuristics. **Addresses:** #4, #5, #6, #7, #9, #14

---

## Tasks

### ☐ 3.1 Replay stored features instead of recomputing — L
**Addresses #4.** The platform records versioned feature values at their
`available_time`; replay should see exactly those, not recomputed ones.
- Load `market_features` from ClickHouse keyed by `available_time` and feed those
  values to the interpreter; fall back to recompute only when a feature is absent.
- Preserves the platform's versioned-feature invariant and live/replay parity.
- **Files:** `crates/backtest/src/store.rs` (feature load), `sim.rs` (feed path).
- **Verify:** a run asserting stored vs recomputed values match on overlapping
  bars.

### ☐ 3.2 Fix the gap-merge over-reach — S
**Addresses #14.** On continuous markets, two missing days separated by a
*present* day can merge and swallow the present day into the reported range.
- In `gaps::merge_days`, only merge across genuinely non-trading days; don't
  bridge across a present day.
- **Files:** `crates/backtest/src/gaps.rs`.
- **Verify:** test the N+1 / N+3 continuous-market case stays two ranges.

### ☐ 3.3 Tick / quote replay — L (optional, larger fidelity)
**Addresses #5.**
- Feed `QuoteTick`/`TradeTick` from `market_trades` when a strategy subscribes to
  those lanes, not just OHLCV bars; route them through the SDK.
- **Files:** `crates/backtest/src/store.rs`, `sim.rs`; SDK may need a quote/trade
  intake helper (market_simulator `sdk.rs`).

### ☐ 3.4 Validate warm-up empirically — M
**Addresses #9.**
- Replace the 5×period / period+1 / floor-30 heuristics with measured indicator
  convergence (or expose them as config) and document the rationale.
- **Files:** `crates/backtest/src/requirements.rs`.

### ☐ 3.5 Broaden & pin down the strategy surface — M
**Addresses #6, #7.**
- Add `PercentOfBalance` / `RiskUnit` sizing support (currently parse-only).
- Decide whether v1.5 universe nodes (Rank/Filter/TakeTopN/DataSource/Surface)
  are in scope for backtest; document the supported surface explicitly.
- Pin the signal rising-edge semantics against a known live case (ties to 4.2).
- **Files:** `crates/backtest/src/sim.rs`, docs.

---

## Definition of Done
Indicator values match the live/stored feature pipeline; gap ranges are exact;
warm-up is justified; the supported strategy surface is documented and broadened
where intended; (optionally) tick replay available.
