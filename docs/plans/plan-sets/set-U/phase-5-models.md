# Phase 5 — Models

**Completion: 63% (5 / 8 tasks).** U-5.1, U-5.3, U-5.4, U-5.6, U-5.7 DONE 2026-10-02; U-5.2 and U-5.5 PARTIAL (embeddings unserved; the ClickHouse series unrun against a server); U-5.8 added (one model per walk-forward fold).

**Requirement IDs:** SU-MOD-01…07.
**Spec:** [FEAT-007](../../../specs/FEAT-007-unified-strategy-engine.md) §8; ADR-0043; FEAT-006 §5, §6.
**Depends on:** Phase 0 (contracts); Phase 1 (request/resume).
**Blocks:** model-driven acceptance in Phases 3, 4, 8.
**Migration range:** 0146 used. 0145 was taken by another session's `0145_monitor_app_grants.sql` while this phase ran; Phase 6 took 0147–0148.

Any model in MLOps, any shape of answer, the right model at the right time. When this phase closes a strategy can chain a classifier, a distributional forecaster and an LLM, backtest them point-in-time, and paper-trade them — none of which is possible today.

---

### U-5.1 Contracts on model versions — M
`DONE` 2026-10-02 — migration 0146 adds `io_contract`, `contract_source` (`derived`|`declared`, CHECKed
paired) and `trained_through` to `model_versions`. `model_registry::contracts::backfill` gives every
derivable version the contract its serving path honours (`domain::model_def::contract::
derived_contract`, using the bundle's `feature_order` when the trainer reported it) — at platform
start and on every newly trained version — and names the ones it cannot (a `transform`). The
compiler reads `backtest::models::RegistrySnapshot`: `name@version` exactly, `name@alias` resolved
to its version *at compile time*. A version with no contract is refused as `model_without_contract`
with its fix, distinct from `unknown_model`. Evidence: `model_registry::contracts` unit tests and
`after_the_backfill_every_derivable_version_has_a_contract` on a scratch PostgreSQL 17 with
0017/0018/0146 (derived, transforms named, idempotent, compiler resolves the pinned version);
`backtest::models` tests (pin and alias; contractless refused, unknown still unknown).

Every model version stores its `IoContract`; existing versions are backfilled with derived contracts; the registry serves contracts to the compiler.

**Files:** `crates/model-registry/src/contracts.rs`, migration 0145.
**Acceptance:** every existing model version has a contract after migration; the compiler resolves a contract by pinned version; a model with no contract cannot be referenced by a v3 strategy (refused with a fix).
**Traces:** ADR-0043 decision 1.

### U-5.2 Serving protocol v2 — L
`PARTIAL` 2026-10-02 — `POST /v2/predict` (`apps/model-inference/app/v2.py`): named feature rows in,
named outputs out as plain JSON, typed in Rust against the contract
(`model_registry::provider::outcome_of`; an output a kind does not produce is that field
`missing`, a malformed one abstains the answer). A row missing a feature — or carrying a NaN —
abstains naming it; an artifact with no feature order is `unavailable`, never guessed. **The v1
zero-fill is gone**: `features_matrix` raises, and `/predict` answers 422 naming the feature
instead of a flat forecast. Classifiers return `probs` (+ `class`), regressors their raw `value`,
rankers and sizers their score. Evidence: `apps/model-inference/tests/test_v2.py` (real LightGBM
models); `crates/model-registry/tests/live_sidecar_v2.rs` run against the service on localhost —
a real regressor answered a typed number and a short row abstained naming `atr_14`.
**Not met:** "an embedding returns its `vec`" — there is no embedding predictor in the inference
service, so an embedding version answers `unavailable` by name. Close with an embedding predictor
that returns `{"embedding": [...]}`; the Rust side already types `vec`.

The sidecar accepts typed named inputs and returns typed named outputs per contract; a missing required input is an abstain with a reason; the zero-fill in `features_matrix` is removed. v1 `Forecast` responses remain for derived forecaster contracts.

**Files:** `apps/model-inference/app/{schemas,predictors/base}.py`, `crates/model-registry/src/sidecar.rs`.
**Acceptance:** a request missing a required feature abstains naming it (the `0.0` fill is gone — a test sends an incomplete row); a classifier returns its full `probs`; a regressor returns its raw `num`; an embedding returns its `vec`.
**Traces:** ADR-0043 decision 3; FEAT-007 §1 defect 4.

### U-5.3 Gateway model provider — M
`DONE` 2026-10-02 — `model_registry::provider::gateway::GatewayModels`: answers the pinned version
a request carries, from the registry snapshot taken when the deployment started; one v2 call per
pinned version per tick; `LiveModels` moved to `model-registry` with `answer_batch`, and the live
runner groups a tick's requests by version, batches concurrently, each batch under its tightest
contract timeout. Wired per deployment in `live_deployments::build`; LLM cost accounting is in
U-5.6. Evidence: `an_alias_moved_after_deployment_does_not_change_the_version_called`;
`requests_to_one_version_in_one_tick_go_in_one_call` (four requests, two versions, two calls).

The paper/live `ModelProvider`: pinned version, batched per tick across a deployment's requests, per-contract timeout, abstain mapping, cost accounting for LLMs.

**Files:** `crates/model-registry/src/provider/gateway.rs`.
**Acceptance:** an alias moved after deployment does not change the version a running deployment calls; requests in one layer go in one batch call (counted).
**Traces:** ADR-0043 decisions 4, 5.

### U-5.4 Backtest model provider — point-in-time — L
`DONE` 2026-10-02 — `backtest::models::PitModels`: a version answers a request at `t` only when its
`trained_through + embargo < t` (embargo = contract `pit.embargo`); unknown `trained_through`
answers no backtest. Cached by (version, instant, digest of the assembled inputs); only answers
and abstains are cached. `optimistic_models` is `UnsafeFlags::models_optimistic` — part of the
`run_id`, skipped from the hash when off so every old id holds — and Gate 0's
`gates::model_leakage` rejects any run that consumed a future-trained answer, naming the first
timestamp. The trainer reports `trained_through_ns` (latest training **or calibration** row,
`app/pit.py`); `ModelManager` records it. The production executor uses it through a
`ModelsFactory` (sidecar source, ClickHouse cache). Evidence, `backtest/tests/v3_models.rs`: a
model trained on the whole window answers nothing in it and the manifest counts every hold as
`model_unavailable`; unknown training end answers nothing; a midway model answers exactly after
its cutoff; the optimistic run is `unsafe`, has its own `run_id`, and fails Gate 0 naming the
first timestamp; a repeated run calls the source zero times with an identical journal digest. A
mutation removing the rule fails three of them.

For a request at t, use the walk-forward fold whose `train_end + embargo < t`; with none, `missing(model_unavailable)`. Batched per (model, fold); results cached as a prediction series. `optimistic_models` exists only as a declared, flagged policy. A G0 check fails a run that consumed any prediction from a fold trained past t − embargo, naming the first timestamp.

**Files:** `crates/model-registry/src/provider/backtest.rs`, `crates/backtest/src/gates/`.
**Acceptance:** a model trained on the whole window cannot answer inside it by default (the strategy holds and the report counts it); a deliberately leaky variant fails G0 with the first offending timestamp; repeating a run reads the cache and calls the sidecar zero times.
**Traces:** ADR-0043 decision 5; FEAT-005 §4.3; FEAT-006 §6; QA-2.

### U-5.5 Prediction series job — M
`PARTIAL` 2026-10-02 — `clickhouse/09_model_prediction_series.sql` (ReplacingMergeTree on
version, instant, inputs digest); `backtest::models::ClickHouseCache` is the backtest provider's
cache, read before the sidecar and written after; `runner::predict_series` runs the engine's own
loop over a graph that only calls the model, so the series is keyed exactly as a later strategy
asks; `jobs::kinds::predict_series` (manifest, `SeriesRunner`, worker) with the platform's runner,
registered in a new Trainer worker pool (no trainer-class job was claimed before). Evidence:
`a_series_covers_exactly_the_out_of_sample_window_and_a_backtest_then_asks_nothing` — rows only
after training end + embargo, one per decision, every out-of-sample bar covered, the in-sample
ones refused; then a different strategy over the window asked the source nothing.
**Not met as written:** the ClickHouse table, cache and job have not run against a ClickHouse
server (none here); the logic is verified through the in-memory store. And "each fold to its own
test window" assumes one model per fold, which training does not produce — see U-5.8.

The `predict_series` job kind applies each fold to its own test window and writes `model_prediction_series` in ClickHouse; the backtest provider reads it before calling the sidecar.

**Files:** `crates/jobs/src/kinds/predict_series.rs`, `clickhouse/08_prediction_series.sql`.
**Acceptance:** a series covers exactly the folds' test windows with no overlap; a backtest over that window makes no sidecar calls.
**Traces:** FEAT-006 §6.

### U-5.6 LLM outputs as typed values — M
`DONE` 2026-10-02 — `model_registry::provider::llm`: the reply's JSON object (bare or fenced) is
typed strictly against the contract's outputs; anything else is `missing(model_abstained)` naming
why. The prompt is told the exact fields to return. `LlmCaps` (per call, per UTC day) refuse a
call whose estimate would break either, before it is made; actual cost is added to the day's
spend. `GatewayModels::with_llm` routes `external_llm_adapter` versions; the adapter rides on the
registry entry; wired per deployment. *Deviation:* the parse lives in the provider, not in
`strategy-engine/src/models.rs` — the engine stays free of text formats; what reaches it is
already a typed value. Evidence: `a_sentiment_field_drives_a_threshold_rule_end_to_end` (0.82 >
0.5 enters; a non-JSON reply holds with the reason in the trace);
`past_the_cost_cap_the_model_abstains_and_the_trace_says_why` (the call is never made); parse and
cap unit tests.

An LLM adapter's contract declares a JSON output schema; `parse(text, schema)` produces a typed `record` or `missing(model_abstained)` on a parse failure; per-call and per-day cost caps abstain rather than overspend.

**Files:** `crates/strategy-engine/src/models.rs`, `crates/model-registry/src/provider/`.
**Acceptance:** a reply that does not match the schema is missing, never a default; a sentiment field drives a threshold rule end to end; exceeding the cost cap abstains and is journaled.
**Traces:** ADR-0043; FEAT-007 §5.3.

### U-5.7 Classifier, regressor and transform kinds — M
`DONE` 2026-10-02 — the trainer now trains a `regressor` as a regression (it fell through to
classification); v2 serves classifier `probs` and regressor `value`. `transform`:
`domain::model_def::transform::TransformSpec` on the definition; the inference service runs it
per row (`v2.run_transform`, restricted builtins, no imports — accident-proofing, not a sandbox:
transforms are owner-registered code, the trust of a pickled artifact);
`contracts::register_transform` dry-runs it against its declared contract before recording it,
and `POST /api/models/transforms` is its door. Transforms take numeric outputs of other nodes
(chaining), have no training end, and answer every backtest decision. Evidence:
`a_classifiers_probabilities_drive_a_strategy`, `a_regressors_value_drives_a_strategy`,
`a_transform_is_called_with_its_code_and_no_artifact` (paper path),
`a_transform_has_no_training_end_to_be_held_to_and_answers_every_decision` (backtest path),
`registering_a_transform_dry_runs_it_against_its_contract`, and
`a_transform_is_registered_typechecked_and_callable` against the live service and a scratch
PostgreSQL (registered, a wrong-shaped transform refused, a strategy compiles against it); Python
`test_kinds.py`, `test_v2.py`.

### U-5.8 One model per walk-forward fold — L
`NOT STARTED`

Found building U-5.4/U-5.5. Training fits **one** model per version, on the first fold's training
window (`worker.py`: `req.folds[0]`), so a version answers point-in-time only after that window —
the rest of a long backtest holds. A walk-forward model answers each fold's test window with the
fold trained just before it. Train every fold, record each fold's artifact and `trained_through`,
and let `PitModels` choose the latest fold trained early enough for each decision.

**Files:** `apps/model-trainer/app/worker.py`, `crates/model-registry/src/manager.rs`, migration,
`crates/backtest/src/models.rs`.
**Acceptance:** a version with *k* folds answers the test window of every fold with that fold's
model; no decision is answered by a fold trained after it; the series job covers the union of the
folds' test windows with no overlap.
**Traces:** FEAT-006 §6; ADR-0043 decision 5.

Generic `classifier` and `regressor` kinds train and serve through the existing families with contracts; `transform` registers a custom code function with a declared contract and no training, called like any model — the escape hatch for anything the operator library cannot express.

**Files:** `crates/domain/src/model_def/kinds.rs`, `apps/model-trainer/`, `apps/model-inference/`.
**Acceptance:** a trained classifier's `probs` and a regressor's `num` are consumed by a strategy; a transform function is registered, typechecked against its contract, and called in backtest and paper.
**Traces:** ADR-0040 decision 5; ADR-0043.
