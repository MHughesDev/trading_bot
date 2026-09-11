# BS-007 / 07 — One feature engine

**Spec status:** spec-ready draft · **Req prefix:** FE- · **Formal spec:** DATA-006 ·
**ADR:** ADR-0029 (single feature engine) · **Plan set:** M · **Gaps fixed:** G-01, G-02

---

## 1. Problem

Four paths compute features, and they disagree (01 §5):
- the backtest supports only `ema_N` and `rsi_N`;
- `crates/features` has about 35 names;
- the Python trainer silently skips unknown names;
- the API zero-fills unknown names.

Models train and serve on different inputs, and most hypotheses can't be backtested.

## 2. Design

- **One Rust crate is the single source of truth.** `crates/features` grows to wrap:
  - the Nautilus indicators already in the market_simulator fork (about 40:
    SMA/EMA/HMA/WMA/VWAP/DEMA/AMA/VIDYA, MACD, RSI, Stochastics, Bollinger, CCI, Aroon,
    Ichimoku, ATR, Donchian, Keltner, variance ratio, efficiency ratio, OBV, book
    imbalance, …);
  - the platform's own families (§4).
- **Two execution modes, one definition:**
  - *incremental* (streaming; used by backtests and live);
  - *batch* (vectorised over a window; used by the Data API, datasets and the agent).
  A parity test asserts they produce identical values, bar for bar.
- **Python bindings** (`tbot_features`, via PyO3) are installed in the agent container,
  the research-runner and the trainer. Research, training, backtest and serving call the
  same code.
- **Deleted:** `apps/model-trainer/app/features.py` feature computation and
  `crates/api/src/features_compute.rs`. Backtest requirement derivation
  (`crates/backtest/src/requirements.rs`) asks the engine instead of pattern-matching
  names.
- **Fail closed:** an unknown feature is an error on every path. Nothing is skipped or
  zero-filled.

## 3. Expression-defined features

- **Features are expressions** in the same expression language as Strategy Language v2
  (08 §4). Examples:
  - `rolling_std(logret(close), 20) / rolling_std(logret(close), 100)`
  - `pct_rank(rolling_max(high, 40) - rolling_min(low, 40), 250)`
  - `lookback_ensemble(sign(close - ema(close, n)), n=[10,20,40,80])`
- Each expression is **parsed, type-checked and content-hashed**. It is registered to a
  project library, and can be promoted to the global library through review. The agent
  invents features without platform code changes.
- **Metadata per feature, derived by the engine:**
  - lookback and warm-up bars;
  - finalisation rule (close-stamped, never intrabar-anticipating);
  - input lanes;
  - output type;
  - NaN policy.
  The engine enforces warm-up. Nothing relies on documentation.

## 4. Families

| Family | Contents | Notes |
|---|---|---|
| Price and returns | returns, log returns, lags, `x[k]` indexing, rolling mean/std/min/max/quantile, z-score, pct_rank, EWMA | Base operators |
| Volatility (range) | Parkinson, Garman–Klass, Rogers–Satchell, **Yang–Zhang** | Drift-robust estimators |
| Realised measures | RV, bipower variation (BV), jump share (RV−BV)/RV, **realised quarticity (RQ)** | RQ feeds HARQ (10 §3; 11 T7). Crypto RV from 5m, 24/7 |
| Persistence and memory | Hurst (R/S and DFA on returns and log-RV), variance ratio, efficiency ratio, autocorrelation, fractional differencing | Regime inputs |
| Trend and oscillators | Nautilus indicators (MA family, MACD, RSI, Stochastics, ADX, Aroon, Donchian, Keltner, Ichimoku) | Via wrapper |
| Liquidity and cost | **EDGE spread estimator**, Roll and Corwin–Schultz (cross-checks), Amihud illiquidity, relative volume, OBV | EDGE drives cost models (09 §5; 11 T10) |
| Order flow | Signed volume, trade-size distribution, **integrated multi-level OFI** | Only where trades or L2 are collected (06 §6) |
| Calendar | hour and day-of-week sin/cos, session flags | — |
| Regime | Filtered labels and probabilities from regime model artifacts (HMM, statistical jump models) | Filtered only, never smoothed (10 §3) |
| Model outputs | `prediction(series_ref).field`: reads a walk-forward prediction series (10 §5) | PIT by construction |
| Text | Volume, novelty and sentiment series from quarantined-reader extractions | Contamination label carried (06 §7) |
| Path signatures (later) | `sig(level, window, transform=lead_lag)` | Must beat plain lags in ablation (research/R2 §7) |

## 5. Interfaces

- **Rust:** a `FeatureSpec` (parsed expression) produces an `IncrementalFeature` or a
  `BatchFeature`. The registry resolves names, library ids and hashes.
- **Python:** `tbot_features.compute(spec_or_name, frame) -> Series` and
  `tbot_features.describe(spec)` (lookback, warm-up, lanes, hash).
- **Data API:** `features(spec, instr, tf, window)` (06 §2). The **SDK:**
  `data.feature("…")` in Layer 1 strategies (08 §3).
- **CLI:**
  - `tbot features list [--family]` lists names, formulas, warm-up and which paths
    support each (all paths, once this spec ships);
  - `tbot features register "<expr>" --name …`.

## 6. Migration

1. Implement the engine and families. Port the existing ~35 names with identical
   semantics, then add Nautilus wrappers.
2. Map the legacy names (`ema_21`, `rsi_14`, `fs_extended_v1` members) to canonical
   expressions, with an alias table.
3. Switch the backtest, trainer and serving to the engine. Delete the legacy
   implementations.
4. Existing model bundles: re-validate `feature_order` against the engine. A bundle whose
   features were zero-filled during training is flagged `requires_retrain`.

## 7. Requirements

| ID | Requirement | Pri |
|---|---|---|
| FE-01 | Exactly one feature implementation (Rust) MUST serve the backtest, live, training, the Data API and the agent | MUST |
| FE-02 | Incremental and batch modes MUST be bit-identical on a parity suite across all families | MUST |
| FE-03 | Unknown features MUST fail closed on every path; silent skipping and zero-filling MUST be removed | MUST |
| FE-04 | Expression features MUST be parsed, type-checked, content-hashed and registrable per project or globally | MUST |
| FE-05 | The engine MUST derive and enforce warm-up, lookback and finalisation per feature | MUST |
| FE-06 | PyO3 bindings MUST be installed in the agent image, research-runner and trainer | MUST |
| FE-07 | The §4 families MUST be implemented, except Order flow (MUST once lanes exist) and Path signatures (MAY) | MUST |
| FE-08 | Legacy feature names MUST resolve through an alias table; affected model bundles MUST be flagged | MUST |
| FE-09 | `tbot features list` MUST report per-feature formula, warm-up and supported paths | MUST |

## 8. Acceptance criteria

1. A strategy using `yang_zhang(20)`, `pct_rank(…)` and an expression feature backtests,
   trains and serves with identical values on all three paths.
2. Requesting an unknown feature fails with a teachable error on every path.
3. The parity suite passes for every family on 3 instruments × 3 timeframes.
4. The legacy `fs_extended_v1` model is flagged `requires_retrain` if its training
   dropped features.
