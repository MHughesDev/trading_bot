# DATA-006: Unified Feature Engine

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0029 (single engine, PyO3, fail-closed); builds on ADR-0008 (same
builders live and replay), ADR-0002 (money never f64; features are f64 statistics)
**Derived from:** BS-007 [07_FEATURES](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/07_FEATURES.MD)
**Plan set:** M (Engine truthfulness)
**Crates:**
- `crates/features` (grows into the engine);
- new `crates/features-py` (PyO3, wheel `tbot_features`);
- `crates/backtest` (requirements derivation);
- `crates/strategy-runtime` (slot binding);
- `apps/model-trainer` (switches to the wheel);
- `crates/api` (Data API `features`);
- adds the `nautilus-indicators` git dependency at the pinned market_simulator rev;
- `migrations/0040_feature_registry.sql`.

**Deletes:**
- feature computation in `apps/model-trainer/app/features.py`;
- `crates/api/src/features_compute.rs`;
- hard-coded EMA/RSI state in `crates/backtest/src/sim.rs` and `requirements.rs`.

---

## 1. Purpose

Exactly one implementation computes every feature on every path:
- backtest;
- live;
- training;
- serving;
- the Data API;
- the agent's sandbox.

Values are bit-identical between streaming and batch modes. Unknown features are errors
everywhere. Features can be defined as expressions without platform code changes.

## 2. Current state (BS-007 01 §5)

| Path | Code | Computes | Unknown names |
|---|---|---|---|
| Backtest | `crates/backtest/src/requirements.rs`, `sim.rs` | `ema_N`, `rsi_N` | Rejected |
| Registry | `crates/features/src/feature_sets.rs` (~35 names), `training_frame.rs`, `ema.rs`, `rsi.rs`, `window.rs` | OHLCV, EMA, RSI, rolling moments, returns, Parkinson, GK, momentum, z-score, relative volume, OBV, calendar | Rejected |
| Trainer | `apps/model-trainer/app/features.py` | Subset | **Silently skipped** |
| API | `crates/api/src/features_compute.rs` | Subset | **Zero-filled** |

## 3. Architecture

```
crates/features
  expr/        # parser + type checker for the shared expression language (also used by SLv2, FEAT-004)
  ops/         # operator library: arithmetic, math, rolling, lag, rank, ensemble, calendar …
  families/    # vol_range, realised, persistence, liquidity, orderflow, regime, prediction, text, nautilus
  plan.rs      # FeatureSpec → DAG of nodes (shared sub-expressions deduplicated)
  incr.rs      # IncrementalEngine: push(bar|trade|book|prediction) → Option<values>
  batch.rs     # BatchEngine: compute(frame, specs) → columns (vectorised, polars-compatible Arrow)
  registry.rs  # names, aliases, library expressions, content hashes, metadata
  parity.rs    # parity harness (test support)
crates/features-py  # PyO3: tbot_features.compute / describe / plan
```

- **One definition, two executors.** Every operator implements both an `IncrOp`
  (constant-memory state, `update(input) -> Option<f64>`) and a `BatchOp`
  (`apply(&[f64]) -> Vec<Option<f64>>`).
- **The batch executor is proven equal to the incremental one.** It may use vectorised
  kernels, but the parity harness compares them bit for bit: exact `f64` equality under
  a fixed summation order. Operators whose vectorised form can't be made bit-identical
  (e.g. Kahan vs naive sums) must use the same algorithm in both.

## 4. Expression language

```
expr   := number | ident | field | call | expr op expr | '-' expr | '(' expr ')' | expr '[' int ']'
field  := open | high | low | close | volume | vwap | trades
call   := ident '(' args ')'      ; args may include keyword args: n=20, kind="yz"
op     := + - * / ^ ; comparisons and boolean ops are SLv2-only (FEAT-004)
```

- **Types:** `series<f64>` (per bar), `scalar`, `bool_series` (SLv2 only). Lookback
  indexing `x[k]` is sugar for `lag(x, k)`.
- **Parameters:** in SLv2, `param` identifiers are materialised to literals before
  planning (as in ADR-0023 v1.2), so each parameter set gets its own plan hash.
- **Canonical form:** the expression is normalised (argument order, keyword defaults
  filled, whitespace removed). `feature_hash = sha256(canonical_form ‖ engine_version)`.
- **Names:** built-in names (`ema_21`, `rsi_14`, …) are aliases to canonical expressions
  (`ema(close, 21)`) in `registry.rs`. Library expressions registered by users or agents
  have a name, scope and hash.

## 5. Operator and family catalogue (v1)

| Group | Operators / families | Notes |
|---|---|---|
| Arithmetic and math | `+ - * / ^`, `abs log exp sqrt sign clip min max` | NaN-propagating |
| Returns | `ret(x,k)`, `logret(x,k)`, `lag(x,k)`, `diff(x,k)` | — |
| Rolling | `rolling_mean/std/var/min/max/sum/median/quantile/skew/kurt(x, n)`, `ewma(x, halflife)`, `zscore(x, n)`, `pct_rank(x, n)` | Window n bars; rank uses stable tie-breaking |
| Nautilus wrappers | `sma ema hma wma dema ama vidya vwap rma lr`, `macd rsi stoch cci aroon cmo roc bb obv kvo psl vhf dm bias pressure ichimoku`, `atr donchian keltner kp rvi vr`, `efficiency_ratio`, `book_imbalance` | Via the `Indicator` trait, fed bars (or book deltas for `book_imbalance`) |
| Range volatility | `parkinson(n)`, `garman_klass(n)`, `rogers_satchell(n)`, `yang_zhang(n)` | Annualisation is explicit: `annualize(x, periods)`; nothing annualises silently |
| Realised measures | `rv(n, sub="5m")`, `bv(n, sub)`, `rq(n, sub)`, `jump_share(n, sub)` | Computed from sub-bars inside each bar; 24/7 crypto aware |
| Persistence | `hurst(x, n, method=rs|dfa)`, `variance_ratio(x, q, n)`, `autocorr(x, lag, n)`, `fracdiff(x, d, thresh)` | — |
| Ensemble | `lookback_ensemble(f, n=[…])` | Equal-weight mean over the parameter list |
| Liquidity and cost | `edge_spread(n)`, `roll_spread(n)`, `cs_spread(n)`, `amihud(n)`, `rel_volume(n)` | EDGE per Ardia–Guidotti–Kroencke; feeds cost models (FEAT-005 §5) |
| Order flow | `signed_volume(n)`, `trade_size_q(q, n)`, `ofi(levels, n)` (integrated multi-level) | Only where trades or L2 are collected (DATA-005) |
| Calendar | `hour_sin/cos`, `dow_sin/cos`, `session(name)` | Instrument calendar aware |
| Regime | `regime(model_ref).prob[k]`, `regime(model_ref).label` | Reads a filtered regime-label artifact; PIT |
| Prediction | `prediction(series_ref).<field>` | Reads a walk-forward prediction series (FEAT-006 §6) |
| Text | `text(series_ref).<field>` | Reads a text-signal series; carries a contamination label |
| Signatures (later) | `sig(level, window, transform=lead_lag)` | MAY; must pass ablation |

**Legacy migration:** every current `known_features_static()` name maps to an alias
with identical semantics. A golden test compares the old and new values for each alias
over 3 instruments × 3 timeframes before the legacy code is deleted.

## 6. Metadata and PIT guarantees (engine-derived)

Each planned feature exposes:
- `lookback` (bars needed before the first valid value);
- `warmup` (bars before values are stable, e.g. EMA convergence, declared per op);
- `inputs` (lanes: bars, trades, book, prediction, text);
- `finalisation = close_stamped` (a value for bar *t* uses only data with
  `available_time ≤ close(t)`);
- `output_type`;
- `nan_policy`;
- `feature_hash`.

**Guarantees:**
- Values are emitted only after `lookback + warmup` bars. Earlier values are `None`, never
  0.
- No operator reads bar *t+1* data when producing *t* (enforced structurally: an
  incremental operator only sees pushed inputs; the batch executor is proven equal).
- **Truncation self-test** (the leakage harness, `leakage_harness.rs`, extended): for
  random truncation points, batch values up to the cut equal the values computed on the
  full frame.

## 7. Registry and storage (`migrations/0040_feature_registry.sql`)

```sql
CREATE TABLE feature_library (
  feature_id TEXT PRIMARY KEY,            -- 'feat_' || first 24 hex of feature_hash
  name TEXT NOT NULL, scope TEXT NOT NULL CHECK (scope IN ('builtin','global','user','project')),
  owner_user_id UUID, project_id UUID,
  canonical_expr TEXT NOT NULL, feature_hash TEXT NOT NULL UNIQUE, engine_version TEXT NOT NULL,
  metadata JSONB NOT NULL,                 -- §6
  description TEXT, created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX feature_name_scope ON feature_library(name, scope,
  coalesce(owner_user_id,'00000000-0000-0000-0000-000000000000'), coalesce(project_id,'00000000-0000-0000-0000-000000000000'));
```

- **Built-ins** are loaded at boot from `registry.rs`. Promotion to global needs review,
  like skills but lighter: parity plus a truncation test plus a human approval.
- **Engine version:** a change to any operator's numerics bumps `engine_version`.
  Artifacts record it, and model bundles pin it.

## 8. Interfaces

**Rust:**

```rust
pub fn plan(specs: &[FeatureSpec]) -> Result<Plan, FeatureError>;         // resolves names/aliases, dedups DAG
pub struct IncrementalEngine { /* … */ }                                  // Plan → per-instrument state
impl IncrementalEngine { pub fn push_bar(&mut self, bar: &Bar) -> Row; /* push_trade, push_book, push_prediction */ }
pub fn compute_batch(plan: &Plan, frame: &ArrowFrame) -> Result<ArrowFrame, FeatureError>;
pub fn describe(spec: &FeatureSpec) -> Result<FeatureMeta, FeatureError>;
```

**Python (`tbot_features`):**
- `compute(spec_or_name, frame: polars.DataFrame) -> polars.Series`;
- `compute_many(list, frame)`;
- `describe(spec) -> dict`;
- `plan(list) -> dict` (DAG, lookback).

**Consumers:**

| Consumer | Change |
|---|---|
| Backtest | `derive_requirements` calls `plan()` for the strategy's feature set and loads `max(lookback + warmup)` bars of warm-up. `sim.rs` feeds bars to an `IncrementalEngine` and writes values into `StrategyInstance` slots (FEAT-005 §4) |
| Live | Hot-path stage 3 uses the same `IncrementalEngine` (when G-11 is wired) |
| Trainer | `features.py` computation is replaced by `tbot_features.compute_many`. The bundle's `feature_order` records the `feature_hash` values and `engine_version` |
| Serving | Inference builds vectors through the same engine. A missing feature is an error, never zero |
| Data API | `GET /api/data/features` uses `compute_batch` over the Data API frame (DATA-005 §5) |
| Agent | `data.feature("…")` in Layer 1 strategies; `tbot features list|register` |

**Errors:** `FeatureError::{Unknown{name, suggestions}, Type{…}, Arity{…},
MissingLane{lane}, Warmup{needed, available}}`. Each maps to the teachable error
envelope `{code, field, rule, fix}`.

## 9. Model bundle migration

- For each existing model bundle, recompute its `feature_order` names through the
  engine.
- If training used the Python path and any requested feature was skipped or zero-filled
  (compare the definition's requested feature set against the bundle's
  `feature_order`), mark the model version `requires_retrain` and exclude it from
  production aliases until it's retrained. This also surfaces in the MLOps UI.

## 10. Test plan and acceptance

| # | Test | BS-007 IDs |
|---|---|---|
| F1 | Parity: incremental == batch, bit for bit, for every operator, on 3 instruments × 3 timeframes × 2 parameter sets | FE-02 |
| F2 | Golden: every legacy name's value is unchanged vs the current implementation (tolerance 0 where the algorithm is identical, 1e-12 otherwise, documented) | FE-08 |
| F3 | Unknown features fail on backtest, trainer, serving, the Data API and the SDK with the same error code | FE-03 |
| F4 | Truncation self-test passes for every operator | FE-05 |
| F5 | A strategy using `yang_zhang(20)`, `pct_rank(rolling_max(high,40)-rolling_min(low,40),250)` and a registered expression backtests, trains and serves with identical values | FE-01, FE-04 |
| F6 | `tbot features list` shows formula, lookback, warm-up and supported lanes | FE-09 |
| F7 | The `fs_extended_v1` model is flagged `requires_retrain` if its training skipped features | FE-08 |
| F8 | The PyO3 wheel imports in the agent image and the trainer image; compute speed is within 2× of native Rust on 1M rows | FE-06 |

## 11. Open questions

1. Bit-identical parity vs vectorised speed for rolling quantiles: accept a slower batch
   path for those operators? Recommended: yes.
2. Where sub-bar data for realised measures comes from at 1m base resolution (5m
   sub-bars need 1m inputs, which is fine; 1m RV needs trades).
