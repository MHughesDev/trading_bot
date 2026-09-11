# BS-007 / 09 — Backtest engine integration

**Spec status:** spec-ready draft · **Req prefix:** BT- · **Formal spec:** FEAT-005 (updates
FEAT-002) · **Plan set:** M · **Builds on:** Set K (real `SimRunExecutor`, stores,
parallelism) · **Gaps fixed:** G-03 (with 10), G-04, G-05, G-06, G-14

---

## 1. Scope

Keep the market_simulator engine (the Nautilus fork, pinned rev). Change how strategies
drive it and how `RunConfig` is honoured, and add the run kinds and tiers the agent
needs.

## 2. Widen the SDK facade

market_simulator's `crates/backtest/src/sdk.rs` exposes only market orders, although the
fork already has all order types, a portfolio and about 40 indicators (01 §4). We own
both repos, so extend the facade with:
- limit, stop, stop-limit, trailing-stop, bracket and OCO commands;
- time-in-force;
- multi-instrument (reserved for OQ-040);
- quote and trade data feeds where collected;
- fill-model and fee-model presets;
- latency.

Pin the new rev in `Cargo.toml`.

## 3. Execution model and fill timing (G-14)

- **Verify first:** determine whether a market order submitted in the bar handler fills
  at that bar's close or the next bar's open. Write a test that pins the behaviour.
- **Explicit default policy:**
  - decide on bar *t*'s close-stamped features;
  - fill at bar *t+1*'s open, plus modelled spread and slippage.
  "Fill at *t* close" is available only as a declared, labelled policy, never the
  default.
- The execution policy is part of `RunConfig`, recorded in every run, and shown in every
  report's execution-assumptions block (research/R2 §10).

## 4. Driving strategies

- **Layer 2:** the backtest drives the same `StrategyInstance` used live:
  - bytecode conditions over the slot array;
  - inference-output slots;
  - sizing and exits.
  It replaces the stateless `evaluate_signals` path in `sim.rs:524–615`. This gives
  backtest/live parity by construction.
- **Layer 1:** the `position_series` kind takes a target-position series (08 §3.2) and
  converts position changes into orders under the execution policy.
- **Model inputs:** for backtests, inference slots are fed from walk-forward prediction
  series (10 §5), never from a single model trained on the whole period. Gate 0 rejects
  any prediction whose producing model's training window overlaps its timestamp.

## 5. Costs, fills and `RunConfig`

`SimRunExecutor` must honour every `RunConfig` field (all are ignored today, 01 §4):

| Field | Behaviour |
|---|---|
| `cost_model_ref` | Dated, per-instrument cost model: fees (venue schedule) + spread (**EDGE** estimate from bars, or quoted spread where quotes exist) + slippage/impact (**square-root law** with σ and volume) (11 T10, T11). Costs are on by default; disabling marks the run unsafe |
| `fill_model` | The §3 policy (next-open default, close, VWAP-of-next-bar, limit-touch rules) |
| `sizing_ref` | Sizing from SLv2 or Layer 1 output; fixed size is only one option |
| `seed` | Seeds every stochastic component (fills, synthetic data, MC) |
| `strategy_version` | Content hash checked at run time; a mismatch fails the run |
| `data_snapshot` | Pins data versions (including revisions) for reproducibility |

With these honoured, `CostSweep` and `Seeds` studies produce meaningfully different
members (G-06).

## 6. Throughput tiers

- **Batch runs:** load bars and features once, then evaluate N parameter sets in one job.
  Sweeps get faster, and fidelity racing becomes cheap.
- **Vectorised tier 0:** a fast position-series evaluator for screening. It is calibrated
  against the event engine: rank correlation on a calibration set must pass the FEAT-003
  P5 gate before its results are used for screening, and it is never used for gate
  verdicts.
- **Parallelism:** per Set K, bounded by job-service queues (05 §3).

## 7. Outputs

Every run stores its outputs as artifacts (05 §6) in the existing ClickHouse
`backtest_run_*` tables plus Arrow:
- trades, fills, positions, equity;
- **feature values at every decision**;
- the execution policy and cost breakdown.

This fixes MAE/MFE (currently 0.0) and the 1m annualisation issue (use
expectancy/profit-factor objectives at 1m). It also enables `explain_trade` and the
diagnostics bundle.

## 8. Determinism and synthetic instruments

- Replay determinism: the same `RunConfig` hash gives bit-identical outputs. This is
  tested in CI.
- Backtests run on `synthetic` venue instruments (06 §8) exactly as on real ones. This is
  used by the eval suite and skill verification.

## 9. Requirements

| ID | Requirement | Pri |
|---|---|---|
| BT-01 | The market_simulator SDK facade MUST expose the §2 order types, fee and fill presets and latency | MUST |
| BT-02 | Fill timing MUST be verified by test, and the default policy MUST be decide-at-close / fill-at-next-open | MUST |
| BT-03 | Layer 2 backtests MUST drive `StrategyInstance`; the stateless `evaluate_signals` path MUST be removed | MUST |
| BT-04 | The `position_series` run kind MUST be supported (08 §3) | MUST |
| BT-05 | Every `RunConfig` field in §5 MUST be honoured | MUST |
| BT-06 | Cost models MUST be dated and per-instrument, built from EDGE and square-root impact; costs are on by default | MUST |
| BT-07 | Model-driven backtests MUST read walk-forward prediction series, and Gate 0 MUST reject training-window overlap | MUST |
| BT-08 | Batch runs MUST evaluate N parameter sets per data load | MUST |
| BT-09 | A vectorised tier-0 evaluator SHOULD exist, calibrated before use and never used for gate verdicts | SHOULD |
| BT-10 | Rich run outputs (§7) MUST be stored as artifacts; MAE/MFE MUST be populated | MUST |
| BT-11 | Replay determinism MUST hold per `RunConfig` hash | MUST |

## 10. Acceptance criteria

1. The fill-timing test passes and the policy appears in every run manifest.
2. A `CostSweep` over three cost models yields three different member distributions.
3. A strategy with a stop and target produces bracket orders, and exits at the stop on a
   synthetic path built to hit it.
4. A strategy using a model input backtests on its prediction series. Substituting a
   full-period model fails Gate 0.
5. A 200-member batch sweep runs at least 5× faster than 200 single runs on the same data
   (target, measured).

## 11. Open questions

1. Latency model defaults per venue class.
2. Whether a vectorised tier is worth building before batch runs prove insufficient.
