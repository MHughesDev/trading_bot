# Phase 4 — Strategy Surface & Sizing

**Completion: 4.1 DONE (2026-10-03, track PROD) · 4.2 and 4.4 SUPERSEDED BY SET U (see their blocks — not closed by Set E) · 4.3 open · 4.5 deferred-by-design**

**Goal:** Close the gaps between what the strategy builder lets users express and
what the engine executes, **without ever breaking live/replay parity**
(ADR-0008). **Addresses:** #22, #23, #24, #25.

> **Parity is the hard constraint.** Indicators and sizing live in shared crates
> (`features`, `strategy-runtime`) consumed identically live and in replay. SMA/
> ATR and `PercentOfBalance` are additive and need **no** format change; AND/OR
> conditions and exit nodes change the grammar and need a **v1.5 definition-
> version bump** (locked decision 9: approved) — precedent: the v1.5 universe
> nodes already in `NodeKind`.

---

## Tasks

### ☑ 4.1 [DONE 2026-10-03] SMA / ATR indicators — S–M each — **no format bump**

> Found already built: `crates/features` carries `sma_N` (alias of
> `rolling_mean_N`) and `atr_N` (Wilder, high/low/close) in the registry and the
> windowed runtime, and the legacy simulator already evaluates features through
> that one runtime with full OHLC rows. The only gate was
> `backtest::requirements::parse_feature`, which accepted `ema`/`rsi` alone.
> It now derives `Sma` and `Atr`, validating periods through the runtime's own
> registry bounds (`atr_1`, `sma_0` refused) and taking warm-up from its
> declared lookback. The builder's two "SMA / ATR not yet supported" warnings in
> `frontend/src/utils/toDefinition.ts` are removed. Tests:
> `requirements::tests` (2 new) and
> `utils/__tests__/sma-and-atr-are-saved-without-a-warning.test.ts` (2). The
> `FEATURE_VERSION` bumps in the original text were unnecessary (nothing about
> the existing features changed). The legacy sim is scheduled for deletion by
> Set U U-8.3; on the v3 engine these are `rolling_mean`/`atr_N` nodes.
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

### ⇄ 4.2 [SUPERSEDED BY SET U — live half DONE 2026-09-15; replay half moves to the v3 engine] `PercentOfBalance` sizing — M/L — **live + replay together**

> **2026-10-03 (track PROD), checked against the Set U plan and code.** The v3
> engine carries `SizeSpec::EquityFraction` through `strategy-engine`
> (`decide.rs`, `planner.rs`) — one sizer for backtest, paper and live — and the
> v1→v3 translator (U-7.3, PARTIAL) maps percent-of-balance `place_order` rules
> to it. So the "replay call site" this task was waiting on is the v3 engine's,
> not `sim.rs`'s, and `sim.rs` is deleted by U-8.3. **What Set E does not close:**
> a *stored v1* PoB strategy still hard-errors in `sim.rs` until U-8.5 migrates
> it; do not evaluate one before then. Set E 4.2 is therefore not marked DONE.
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
- **Locked decision 10:** size against **total account equity** (cash +
  open-position value) — the identical figure fed to the shared sizer both live
  and in the SDK handler.
- Clear the builder `percent_of_equity` warning (`toDefinition.ts:167-173`).
- **Files:** `crates/strategy-runtime/src/intents.rs`, `runtime.rs`,
  `crates/backtest/src/sim.rs`, `frontend/src/utils/toDefinition.ts`.
- **Verify:** a PoB strategy produces identical order sizes live and in a
  matching replay fixture (parity test); lot quantization is deterministic.
- **Verified by (live):** `strategy_runtime::intents::tests::percent_of_balance_sizes_against_equity_and_mark`,
  `..._quantizes_down_to_the_lot`, `..._without_context_errors_loudly`,
  `..._that_rounds_to_zero_errors`, and
  `hot_path::tests::percent_of_balance_sizes_a_real_order` end to end.
- **PARTIAL — the replay call site is not wired.** The sizer is in the one
  shared crate (`strategy-runtime::intents::build_intents_for_signals`, which
  now takes a `SizingContext`), so live and replay *can* be byte-identical.
  `crates/backtest/src/sim.rs` still hard-errors on non-`Fixed` and belongs to
  **track 15**; the parity test cannot be written until that call site adopts
  the shared sizer. Until then PoB executes live and not in replay, which is
  an ADR-0008 gap that must be closed before any PoB strategy is evaluated.
- Also changed: the sizer returns `Result<Option<_>>`. `Ok(None)` is a
  deliberate abstention and `Err` is a failure; they used to be the same
  `None`, which is why PoB produced no order **and no error**. `RiskUnit`
  now errors with its reason instead of returning nothing.

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
- **Locked decision 11:** the universe feed is **one shared path** for
  ScannerPanel discovery and future Automations pipelines (not two).
- **Files:** `crates/api/src/routes/strategies.rs`, universe-feed module,
  `frontend/src/components/trading/ScannerPanel.tsx`.
- **Verify:** a discovery strategy surfaces a real instrument set end-to-end;
  the universe feed has a test.

### ⇄ 4.4 [SUPERSEDED BY SET U — the v1.5 bump is replaced by the v3 graph; builder emission still open] Strategy-definition v1.5: exit nodes + AND/OR — L — **format bump**

> **2026-10-03 (track PROD), checked against the Set U plan and code.** Exits
> are first-class in the v3 intent vocabulary (`bracket`, `protect`, trailing and
> `atr(k, n)` distances — `strategy-engine/src/compile.rs`), boolean conditions
> are ordinary expression nodes, and the translator (U-7.3) turns a v1 spec's
> exits into a `protect` rule. A v1.5 grammar bump would be a second format to
> maintain beside it, so it is not built. **What stays true until the builder
> emits v3 (U-7.4):** `toDefinition.ts` keeps *rejecting* a spec that declares
> exits on save rather than saving it without them — that refusal is correct and
> must not be removed early. Set E 4.4 is therefore not marked DONE.
**Addresses #24 (NF).** v1.0 grammar has no boolean operators and no exit nodes
(`nodes.rs`, ADR-0007); the builder hard-errors on AND/mixed conditions and
drops exit rules (`toDefinition.ts:130-165`). Both require grammar/vocabulary
changes → an **additive v1.5 bump** (locked decision 9: approved), mirroring the
existing v1.5 universe-node precedent.
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
- **PARTIAL (track 16).** The format bump is **not** done. What changed is the
  failure mode: a spec declaring exit rules is now **rejected on save**
  (`toDefinition.ts`) instead of saved with a warning and the exits silently
  dropped. A strategy that runs without the stop loss its author declared,
  while the builder still shows the stop, is worse than one that will not
  save — the operator sizes the position believing in a control that does
  not exist. AND/OR mixing was already an error and stays one.
- The executable half is still open and is coupled to **track 15**
  (`ExitTracker` in the simulator): declaring exits without the simulator
  honouring them would make every backtest of an exit-declaring strategy
  wrong in the optimistic direction.

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
