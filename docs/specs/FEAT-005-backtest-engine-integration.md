# FEAT-005: Backtest Engine Integration v2

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0006 (market_simulator as the backtest engine), ADR-0019 (Run atom),
ADR-0026, ADR-0029, ADR-0030
**Derived from:** BS-007 [09_BACKTEST](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/09_BACKTEST.MD)
**Updates:** FEAT-002 (backtesting)
**Builds on:** Set K (real `SimRunExecutor`, stores, parallel execution; the executor
is partly shipped with FEAT-003 Phase 1)
**Plan set:** M (Engine truthfulness)
**Repos and crates:**
- `market_simulator` (`crates/backtest/src/sdk.rs` facade; new pinned rev);
- `crates/backtest` (`sim.rs`, `sim_executor.rs`, `run/*`, new `cost/`, `vector/`);
- `crates/strategy-runtime` (`StrategyInstance`);
- `crates/features` (DATA-006);
- `clickhouse/07_backtest_outputs_v2.sql`.

**Fixes:** G-03 (with FEAT-006), G-04, G-05, G-06, G-14; MAE/MFE = 0; the 1m
annualisation issue

---

## 1. Purpose

Keep the Nautilus-based engine, and make every backtest honest and expressive:
- strategies drive it through the same runtime used live;
- every `RunConfig` field is honoured;
- costs are realistic by default;
- fill timing is explicit;
- outputs are rich enough to explain every trade.

## 2. Current state (verified 2026-09-10)

- **`sim.rs:524–615`:** `build_handler` builds only `IndicatorState::Ema/Rsi` and calls
  the stateless `strategy_runtime::evaluate_signals(&nodes, &feature_values,
  &bar_map)`. Orders are `SimOrderCommand::Market` on a signal's rising edge.
- **The facade exposes one command:** `SimOrderCommand::Market { side, quantity }`
  (market_simulator `crates/backtest/src/sdk.rs:127`).
- **`sim_executor.rs`** resolves the definition by slug and ignores `cost_model_ref`,
  `fill_model`, `sizing_ref`, `seed`, `data_snapshot` and `strategy_version`. The
  `RunConfig` type declares them (`run/config.rs`), and `FillModel` already enumerates
  `NextBarOpen | CurrentClose | LimitProb | PessimisticIntrabar`.
- Outputs go to ClickHouse `backtest_run_equity` and `backtest_run_trades` (`mae`/`mfe`
  columns exist but are written as 0).

## 3. Execution model and fill timing

1. **Verify (G-14), first task of Set M:** a test submits a market order from the bar
   handler at bar *t* and asserts which price it fills at. Record the result in this
   spec.
2. **The default policy is `FillModel::NextBarOpen`:**
   - decisions use bar *t*'s close-stamped features;
   - market orders fill at bar *t+1*'s open ± modelled half-spread and slippage;
   - stop and limit orders trigger on bar *t+1…* high/low paths under the engine's
     intrabar model.
3. **`CurrentClose`** is allowed only as a declared policy. Runs using it carry
   `execution_policy=optimistic_close` in their manifest and verdicts.
4. **`PessimisticIntrabar`** (stop checked before target when both trigger in one bar)
   is the default for bracket exits.
5. The execution policy is part of `RunConfig`, hashed into `run_id`, and printed in
   every report's execution-assumptions block.

## 4. Driving strategies

### 4.1 Layer 2 (`RunKind::Definition`)

- **Setup:** resolve `strategy_version`, and fail with `version_mismatch` if the stored
  AST hash ≠ `RunConfig.strategy_version`. Materialise params, compile (FEAT-004 §3.3)
  and plan features (DATA-006).
- **Per bar:**
  1. push the bar (and trades, book or prediction inputs) into the `IncrementalEngine`;
  2. write values to `StrategyInstance` slots;
  3. `StrategyInstance::on_bar()` evaluates entries, exits, sizing and risk, and emits
     `OrderIntent`s;
  4. translate them to `SimOrderCommand`s.
- **Model slots** are bound to prediction-series artifacts at setup (§4.3).
- This replaces `evaluate_signals`. `StrategyInstance` is the same type the live hot path
  will use (G-11), so parity holds by construction.

### 4.2 Layer 1 (`RunKind::PositionSeries { series_handle }`)

- Load the target-position series (FEAT-004 §2.2).
- At each decision time, compute `delta = target − current`. Emit a market order (or a
  declared order type) sized to `delta × equity / price`, rounded to lot and tick size by
  the engine.
- Costs and fills follow the Run's policy. The series artifact's code hash is in
  `strategy_version`.

### 4.3 Model inputs

- **In backtests, model slots read walk-forward prediction series** (FEAT-006 §6), never
  a model trained on the full window.
- **Gate 0 check:** for each prediction consumed at time *t*, its producing fold's
  `train_end < t − embargo`. A violation fails G0 with the first offending timestamp.
- Abstain policies (`hold | flat | skip_entry`) apply when a series has no value at
  *t*.

## 5. Costs (`crates/backtest/src/cost/`)

**`CostModel`** (a dated artifact per instrument and venue, referenced by
`RunConfig.cost_model_ref = "cost:<instrument>@<date>"`):

```yaml
fees:      { maker_bps: 16, taker_bps: 26, source: "venue schedule <url/date>" }
spread:    { method: edge|quoted|fixed, half_spread_bps: series_ref | value, window: 30d }   # EDGE from bars by default
impact:    { model: sqrt, Y: 0.7, sigma: daily_vol_series_ref, adv: series_ref }           # Δp ≈ Y σ √(Q/V)
borrow:    { bps_per_day: 0 }          # equities short; crypto perps funding handled by FEAT data lanes
latency:   { bars: 0 }                 # decision → order arrival, in bars (0 or 1)
```

- **Built by an `instrument_profile` job** (AGENT-003 §7): EDGE half-spread (rolling),
  ADV, σ and the venue fee table.
- **Costs on by default.** `UnsafeFlags.costs_disabled` marks a run unsafe (exists).
- **`CostSweep` studies** vary `cost_model_ref` across multipliers. They must now produce
  different member distributions.

## 6. `RunConfig` honoured (every field)

| Field | Behaviour |
|---|---|
| `strategy_ref` / `strategy_version` | Resolve by version hash; slug overwrite can't change a run |
| `params` | Materialised before compile or planning |
| `data_slice` | Universe, window, eval resolution, construction; DATA-005 as-of membership for universes |
| `cost_model_ref` | §5 |
| `fill_model` | §3 |
| `sizing_ref` | `definition` = sizing from the AST / series; named sizing presets for Layer 1 (`fixed_fraction`, `vol_target`) |
| `seed` | Seeds every stochastic element (probabilistic fills, MC) |
| `data_snapshot` | Reads bars with `ingested_time ≤ snapshot` (DATA-005 §4); reproducible reruns |

## 7. Throughput

- **Batch runs:** `BatchRun { base: RunConfig, param_sets: Vec<ParamMap> }` loads bars
  and features once, plans the union of feature DAGs, and evaluates the members
  sequentially or in parallel within one worker. Each member is still its own `RunConfig`
  and `run_id`, and is counted (INV-1).
- **Vectorised tier 0** (`crates/backtest/src/vector/`): a fast position-series
  evaluator (bar-close signals, next-open fills, fee plus half-spread costs, no intrabar
  exits). It is used only for screening under the FEAT-003 P5 calibration gate
  (Spearman ρ ≥ 0.8 vs the event engine on a calibration set). It never produces gate
  verdicts.
- **Parallelism:** job-service worker pools (COMP-005 §7). The fixed
  `MAX_CONCURRENT_RUNS` is removed.

## 8. Outputs (`clickhouse/07_backtest_outputs_v2.sql`)

| Table | Columns (additions) |
|---|---|
| `backtest_run_trades` | Populate `mae`, `mfe` (from the intrabar path); add `exit_reason`, `entry_order_type`, `fees_str`, `spread_cost_str`, `impact_cost_str` |
| `backtest_run_fills` (new) | run_id, order_id, ts_ns, side, qty_str, price_str, liquidity (maker/taker), fee_str |
| `backtest_run_positions` (new) | run_id, ts_ns, qty_str, avg_price_str |
| `backtest_run_decisions` (new) | run_id, ts_ns, slot_values (Map(String, Float64)), signals (Array(String)), intents (String) |

- **Annualisation:** metrics use `periods_per_year` from the instrument calendar (24/7
  crypto: 525,600 at 1m). Objectives at ≤ 5m default to expectancy or profit factor
  (existing FEAT-003 guidance). Reports never annualise 1m Sortino/Calmar without a
  warning.
- **`explain_trade(run_id, trade_idx)`** reads the decisions table for the entry bar and
  returns slot values, regime, path, MAE/MFE and exit reason.

## 9. market_simulator facade changes (separate repo; pinned rev bump)

```rust
pub enum SimOrderCommand {
    Market   { side: OrderSide, quantity: Quantity, tif: TimeInForce },
    Limit    { side, quantity, price: Price, tif, post_only: bool },
    Stop     { side, quantity, trigger: Price, tif },
    StopLimit{ side, quantity, trigger: Price, price: Price, tif },
    Trailing { side, quantity, offset: TrailingOffset, tif },
    Bracket  { entry: Box<SimOrderCommand>, stop: Price, target: Option<Price> },   // OCO children
    Cancel   { client_order_id: ClientOrderId },
}
pub struct SimConfig { /* + */ fee_model: FeeModelPreset, fill_model: FillModelPreset, latency_bars: u8, instruments: Vec<InstrumentSpec> }
```

- **Data inputs:** bars (exists), plus trade ticks and quote ticks where supplied.
- The handler receives fills and order-status events (for `StrategyInstance` state).
- Multi-instrument subscriptions are reserved behind a feature flag until OQ-040.
- These are built on engine capabilities that already exist upstream. The facade is
  what's widened.

## 10. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| B1 | The fill-timing test pins behaviour; the default `NextBarOpen` appears in every run manifest | BT-02 |
| B2 | A `CostSweep` over three cost models yields three different member distributions | BT-05, BT-06 |
| B3 | Stop and target produce a bracket; a synthetic path built to hit the stop exits at the stop with `exit_reason=stop`; MAE/MFE are non-zero | BT-01, BT-10 |
| B4 | A strategy using a model slot backtests on its prediction series; substituting a full-period model fails G0 | BT-07 |
| B5 | Replay determinism: the same `RunConfig` gives bit-identical trades, fills and equity | BT-11 |
| B6 | A 200-member batch sweep is ≥ 5× faster than 200 single runs on the same data (target) | BT-08 |
| B7 | The vectorised tier stays off until calibrated, and is never used for verdicts | BT-09 |
| B8 | v1 → v2 round-trip trades are identical under the same policy (with FEAT-004 S5) | BT-03 |
| B9 | A `position_series` run executes the series with costs; client-supplied P&L is rejected | BT-04 |
| B10 | `strategy_version` mismatch → run fails with `version_mismatch` | BT-05 |

## 11. Open questions

1. Latency defaults per venue class (0 or 1 bar).
2. The default impact coefficient Y per asset class before calibration data exists.
