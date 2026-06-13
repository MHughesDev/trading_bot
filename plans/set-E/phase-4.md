# Phase 4 — Strategy Surface & Sizing

**Completion: 0% (0 / 4 tasks complete; 1 deferred-by-design — 4.5)**

**Goal:** Close the gaps between what the strategy builder lets users express and
what the engine executes, **without ever breaking live/replay parity**
(ADR-0008). **Addresses:** #22, #23, #24, #25.

> **Parity is the hard constraint.** Indicators and sizing live in shared crates
> (`features`, `strategy-runtime`) consumed identically live and in replay. SMA/
> ATR and `PercentOfBalance` are additive and need **no** format change; AND/OR
> conditions and exit nodes change the grammar and need a **v1.5 definition-
> version bump** (Open Decision 9) — precedent: the v1.5 universe nodes already
> in `NodeKind`.

---

## Tasks

### ☐ 4.1 SMA / ATR indicators — S–M each — **no format bump**
**Addresses #23 (NF).** Builder warns SMA/ATR "not yet supported"
(`toDefinition.ts:122-126`); confirmed absent from `crates/features/` (only EMA/
RSI/window). Feature names are free strings on `features.technical`, so no format
change — implement in the shared crate and both live + replay gain them.
- Implement `Sma` (reuse `window.rs`) and `Atr` (needs high/low/close, available
  in `BarPayload`) in `crates/features/`; bump each `FEATURE_VERSION`.
- Register in the backtest handler: extend `FeatureKind` + `IndicatorState`
  match (`backtest/src/sim.rs:88-91,327-337`) and `requirements.rs` parsing.
- Remove the builder warning for SMA/ATR (`toDefinition.ts`).
- **Files:** `crates/features/src/{sma,atr}.rs`, `lib.rs`,
  `crates/backtest/src/{sim,requirements}.rs`, `frontend/src/utils/toDefinition.ts`.
- **Verify:** indicator unit tests; a backtest using `sma_*`/`atr_*` runs;
  builder no longer warns.

### ☐ 4.2 `PercentOfBalance` sizing — M/L — **live + replay together**
**Addresses #22 (NF, PoB).** Parse-only in the shared
`intents::build_intent_from_action` (returns `None`,
`strategy-runtime/src/intents.rs:35`); backtest hard-errors on non-Fixed
(`sim.rs:245`). Must land in the **one shared path** or it's non-deterministic
across live/replay.
- Add `SizingContext { account_equity: Decimal, mark_price: Price }` to
  `build_intent_from_action` / `build_intents_for_signals`. PoB:
  `qty = (fraction * equity / mark_price)` quantized to lot size via the
  existing `Decimal`/`dec_str` path.
- **Live:** thread equity + mark into `process_event` (`runtime.rs:117`) from
  account-snapshot + latest-mark (paper account exposes `equity` at
  `paper/account_source.rs:48-58`).
- **Replay:** replace the Fixed-only guard with the **same** shared sizer, fed
  per-bar equity from the SDK account + bar close as mark.
- Resolve **Open Decision 10** (identical equity figure both sides).
- Clear the builder `percent_of_equity` warning (`toDefinition.ts:167-173`).
- **Files:** `crates/strategy-runtime/src/intents.rs`, `runtime.rs`,
  `crates/backtest/src/sim.rs`, `frontend/src/utils/toDefinition.ts`.
- **Verify:** a PoB strategy produces identical order sizes live and in a
  matching replay fixture (parity test); lot quantization is deterministic.

### ☐ 4.3 Scanner surface endpoint + universe feed — M/L
**Addresses #25 (NF).** `ScannerPanel.handleStrategyChange` only sets state; it
never runs the discovery strategy (`ScannerPanel.tsx:37-40`). The evaluator
exists (`strategy-runtime/nodes/mod.rs:53 evaluate_universe_pipeline`) but is
**only called from tests** — there is no live wiring, no API endpoint, and the
pipeline's `initial_universe` is unsourced outside tests.
- Add `GET /api/strategies/:id/surface`: load the discovery strategy, build the
  universe from the demand-driven feature store, run `evaluate_universe_pipeline`,
  return `{ instruments: [...] }` (ideally with per-instrument score/feature data
  for `WatchTileData`).
- Wire `ScannerPanel` to call it and `setTiles`.
- Resolve **Open Decision 11**: the universe feed is **one shared path** for
  ScannerPanel discovery and future Automations pipelines.
- **Files:** `crates/api/src/routes/strategies.rs`, universe-feed module,
  `frontend/src/components/trading/ScannerPanel.tsx`.
- **Verify:** a discovery strategy surfaces a real instrument set end-to-end;
  the universe feed has a test.

### ☐ 4.4 Strategy-definition v1.5: exit nodes + AND/OR — L — **format bump**
**Addresses #24 (NF).** v1.0 grammar has no boolean operators and no exit nodes
(`nodes.rs`, ADR-0007); the builder hard-errors on AND/mixed conditions and
drops exit rules (`toDefinition.ts:130-165`). Both require grammar/vocabulary
changes → an **additive v1.5 bump** (Open Decision 9), mirroring the existing
v1.5 universe-node precedent.
- Extend the expression grammar (or add compositional condition nodes) for
  AND/OR; add exit node/action types (stop / take-profit / trailing) honoured
  **both live and in the SDK sim**. Update the validator (fail-closed on unknown)
  and the bytecode compiler (`strategy-runtime/src/bytecode.rs`). Bump
  `definition_version` to `1.5` with a migration/compat story for existing v1.0
  defs.
- Emit the new constructs from `toDefinition.ts`; remove the AND/OR + exit
  warnings.
- **Files:** `crates/domain/src/strategy_def/nodes.rs`, `crates/strategy-validator`,
  `crates/strategy-runtime/src/bytecode.rs`, backtest sim exit handling,
  `frontend/src/utils/toDefinition.ts`.
- **Verify:** existing v1.0 strategies still validate/run unchanged; a v1.5
  strategy with AND/OR + a stop-loss runs identically live and in replay.

---

## Deferred-by-design

### ⏸ 4.5 `RiskUnit` sizing — L
**Addresses #22 (RiskUnit).** "R" (entry-to-stop distance) is **undefined until
exit/stop nodes exist** (4.4). Once 4.4 lands, `RiskUnit` can size from the stop
distance via the same `SizingContext` path as 4.2. Blocked on 4.4; do not
attempt before. Keep returning a clear error until then.

---

## Definition of Done
The builder no longer accepts-then-drops features: SMA/ATR and percent-of-balance
sizing execute (with proven live/replay parity); the scanner surfaces real
discovery output through a shared universe feed; and a versioned v1.5 format
supports AND/OR conditions and exit rules without breaking any v1.0 strategy.
`RiskUnit` remains deferred until exit nodes provide a stop distance.
