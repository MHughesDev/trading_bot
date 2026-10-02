# Phase 5 — Models

**Completion: 0% (0 / 7 tasks)**

**Requirement IDs:** SU-MOD-01…07.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §8; ADR-0043; FEAT-006 §5, §6.
**Depends on:** Phase 0 (contracts); Phase 1 (request/resume).
**Blocks:** model-driven acceptance in Phases 3, 4, 8.
**Migration range:** 0144–0146.

Any model in MLOps, any shape of answer, the right model at the right time. When this phase closes a strategy can chain a classifier, a distributional forecaster and an LLM, backtest them point-in-time, and paper-trade them — none of which is possible today.

---

### U-5.1 Contracts on model versions — M
`NOT STARTED`

Every model version stores its `IoContract`; existing versions are backfilled with derived contracts; the registry serves contracts to the compiler.

**Files:** `crates/model-registry/src/contracts.rs`, migration 0144.
**Acceptance:** every existing model version has a contract after migration; the compiler resolves a contract by pinned version; a model with no contract cannot be referenced by a v3 strategy (refused with a fix).
**Traces:** ADR-0043 decision 1.

### U-5.2 Serving protocol v2 — L
`NOT STARTED`

The sidecar accepts typed named inputs and returns typed named outputs per contract; a missing required input is an abstain with a reason; the zero-fill in `features_matrix` is removed. v1 `Forecast` responses remain for derived forecaster contracts.

**Files:** `apps/model-inference/app/{schemas,predictors/base}.py`, `crates/model-registry/src/sidecar.rs`.
**Acceptance:** a request missing a required feature abstains naming it (the `0.0` fill is gone — a test sends an incomplete row); a classifier returns its full `probs`; a regressor returns its raw `num`; an embedding returns its `vec`.
**Traces:** ADR-0043 decision 3; FEAT-007 §1 defect 4.

### U-5.3 Gateway model provider — M
`NOT STARTED`

The paper/live `ModelProvider`: pinned version, batched per tick across a deployment's requests, per-contract timeout, abstain mapping, cost accounting for LLMs.

**Files:** `crates/model-registry/src/provider/gateway.rs`.
**Acceptance:** an alias moved after deployment does not change the version a running deployment calls; requests in one layer go in one batch call (counted).
**Traces:** ADR-0043 decisions 4, 5.

### U-5.4 Backtest model provider — point-in-time — L
`NOT STARTED`

For a request at t, use the walk-forward fold whose `train_end + embargo < t`; with none, `missing(model_unavailable)`. Batched per (model, fold); results cached as a prediction series. `optimistic_models` exists only as a declared, flagged policy. A G0 check fails a run that consumed any prediction from a fold trained past t − embargo, naming the first timestamp.

**Files:** `crates/model-registry/src/provider/backtest.rs`, `crates/backtest/src/gates/`.
**Acceptance:** a model trained on the whole window cannot answer inside it by default (the strategy holds and the report counts it); a deliberately leaky variant fails G0 with the first offending timestamp; repeating a run reads the cache and calls the sidecar zero times.
**Traces:** ADR-0043 decision 5; FEAT-005 §4.3; FEAT-006 §6; QA-2.

### U-5.5 Prediction series job — M
`NOT STARTED`

The `predict_series` job kind applies each fold to its own test window and writes `model_prediction_series` in ClickHouse; the backtest provider reads it before calling the sidecar.

**Files:** `crates/jobs/src/kinds/predict_series.rs`, `clickhouse/08_prediction_series.sql`.
**Acceptance:** a series covers exactly the folds' test windows with no overlap; a backtest over that window makes no sidecar calls.
**Traces:** FEAT-006 §6.

### U-5.6 LLM outputs as typed values — M
`NOT STARTED`

An LLM adapter's contract declares a JSON output schema; `parse(text, schema)` produces a typed `record` or `missing(model_abstained)` on a parse failure; per-call and per-day cost caps abstain rather than overspend.

**Files:** `crates/strategy-engine/src/models.rs`, `crates/model-registry/src/provider/`.
**Acceptance:** a reply that does not match the schema is missing, never a default; a sentiment field drives a threshold rule end to end; exceeding the cost cap abstains and is journaled.
**Traces:** ADR-0043; FEAT-007 §5.3.

### U-5.7 Classifier, regressor and transform kinds — M
`NOT STARTED`

Generic `classifier` and `regressor` kinds train and serve through the existing families with contracts; `transform` registers a custom code function with a declared contract and no training, called like any model — the escape hatch for anything the operator library cannot express.

**Files:** `crates/domain/src/model_def/kinds.rs`, `apps/model-trainer/`, `apps/model-inference/`.
**Acceptance:** a trained classifier's `probs` and a regressor's `num` are consumed by a strategy; a transform function is registered, typechecked against its contract, and called in backtest and paper.
**Traces:** ADR-0040 decision 5; ADR-0043.
