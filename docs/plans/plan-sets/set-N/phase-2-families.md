# Phase 2 — Model families and bring-your-own

**Completion: 60% (6 / 10 tasks), plus 4 `PARTIAL`.**

**Requirement IDs:** MD-04, MD-05, MD-06, MD-07
**Spec:** [FEAT-006](../../../specs/FEAT-006-model-platform-v2.md) §4, §5
**Depends on:** Phase 1.
**Migration range:** 0208.

`worker.py::_route` is a framework switch. FEAT-006 §4 replaces it with a registry keyed by `(family, variant)`, each entry declaring supported targets and outputs, whether it needs sequences and
whether it uses a GPU. Baselines are always trained beside any candidate on the same folds, so a scorecard can never be read without the thing it has to beat.
This phase lives almost entirely in `apps/model-trainer`, which is outside the F-RESERVED territory; the tasks say what they need.

---

### N-2.1 The family registry — M
`DONE` *(2026-10-04, PY1)* `apps/model-trainer/app/families/{registry.json,registry.py}`: 36 entries; `worker._route` delegates to `registry.route` (explicit `definition["family"]` wins, else the legacy framework alias map). Evidence: `tests/test_families_registry.py` 16 passed (unknown pair refused with the known list; `needs_sequence` refused without `sequence`; every legacy framework incl. the quantile variants resolves through `alias_for` + `route`; foundation refused as candidate); `test_families_worker_wiring.py::test_train_request_declaration_and_family_reach_the_train_function`. Mutation: removing the candidate guard fails the registry test. Not here: the Rust-side validation against the same JSON.

A registry keyed by `(family, variant)` replacing `_route`; each entry declares targets, outputs, sequence need and GPU use.

**Files:** `apps/model-trainer/app/families/` (outside this track)
**Acceptance:** an unknown `(family, variant)` is refused with the known list; a family that needs sequences refuses a dataset without `sequence`; every existing framework is reachable through the registry.
**Traces:** MD-06; FEAT-006 §4

### N-2.2 The baseline family: mean, EWMA, ridge, HAR, Log-HAR, HARQ — L
`DONE` *(2026-10-04, PY1)* `families/baseline.py`; `worker.run_evaluation` attaches `metrics["family_baselines"]`, `["fold_ids"]`, `["candidate"]`, `["series"]` (shape in the module docstring; the legacy `metrics["baselines"]` dict is untouched). Evidence: `test_families_baseline.py` 7 passed, `test_families_worker_wiring.py::test_garch_eval_attaches_family_baselines_series_and_uses_conditional_sigma` and `::test_evaluation_without_baselines_is_refused` (`run_baselines=false` gives `data_error`). HARQ without realised quarticity raises `MissingRequirement` (typed `data_error`) and shows as a `refused` row in an evaluation. Mutations: dropping the requirement check, and dropping the no-baseline refusal, each fail a test. Not here: `run_training` does not itself compute baselines (evaluation does); Rust parsing of the rows.

Always trained alongside any candidate on the same folds. HAR and HARQ are the vol baselines the hard usage rule (N-3.2) compares against.

**Files:** `apps/model-trainer/app/families/baseline.py` (outside this track)
**Acceptance:** a candidate's scorecard contains every baseline's scores on identical folds; HARQ needs realised quarticity and refuses without it; a run that trains a candidate and no baseline is refused.
**Traces:** MD-06, MD-09; FEAT-006 §4

### N-2.3 Vol family: GARCH, GJR, EGARCH, realised GARCH — M
`DONE` *(2026-10-04, PY1)* `families/vol.py`, own MLE in numpy/scipy (no `arch`); `adapters/garch_adapter.py` is now an alias onto it. Evidence: `tests/test_families_vol.py` 10 passed: GARCH/GJR/EGARCH fit on a simulated clustered leverage series (planted gamma recovered, GJR loglik beats GARCH), one-step sigma differs from the unconditional one, realised GARCH fits with a realised measure. The reference dataset is a simulated seeded one; no real-data fit was run.

Conditional volatility models behind the family registry; serving uses the conditional forecast (N-4.2).

**Files:** `apps/model-trainer/app/families/vol.py` (outside this track)
**Acceptance:** the fitted model's one-step sigma differs from the unconditional sigma on a clustered series; EGARCH and GJR fit on the reference dataset.
**Traces:** MD-06, MD-12; FEAT-006 §4

### N-2.4 Regime family: HMM and statistical jump models, filtered only — L
`DONE` *(2026-10-04, PY1)* `families/regime.py`: Gaussian HMM (own EM; forward-filtered labels), JM, continuous JM, sparse JM; artifact type `regime_labels` in the bundle header. Evidence: `tests/test_families_regime.py` 11 passed incl. truncation tests at 130 cutoffs per kind, no smoothing or Viterbi symbol exported, sparse JM down-weights a noise feature. Mutation: labelling from the offline DP instead of the forward values fails the truncation test. Caveat: the continuous JM is an online soft-membership approximation (ADR-N-PY1-04).

Gaussian HMM with filtered probabilities and the jump model family (JM, continuous JM, sparse JM). Outputs **filtered** labels only; artifact type `regime_labels`. A
smoothed label uses the future and is refused at the type. (Set R-b RB-4.2 consumes this.)

**Files:** `apps/model-trainer/app/families/regime.py` (outside this track)
**Acceptance:** a label at time t is unchanged when later bars are appended (a truncation test); the smoothed-label path does not exist.
**Traces:** MD-06; FEAT-006 §4

### N-2.5 Sequence models consume real windows — L
`PARTIAL` 2026-10-04 (R-N/PY2) — `app/windows.py`: `declaration.sequence {window, stride}` turns each split into 3-D windows cut inside the split (never across train/val/test, last row = label row), `torch_adapter`/`forecaster` use it, the bundle header carries top-level `sequence {window, stride}`, `windows.predict_bundle` builds the same window from the latest W bars and abstains (`insufficient_history`) below W. Tests (`tests/test_sequence_windows.py`, 29): `bundle_records_sequence_window_64_and_serving_abstains_below_it`, `a_model_on_windows_beats_the_same_model_at_seq_len_1_on_a_lag_series` (test MSE < 0.5x; mutation: dropping `apply_sequence` fails it), `no_window_crosses_a_split_boundary`, `serving_uses_only_the_last_window_of_bars`. **Missing:** production serving (`apps/model-inference/app/predictors/torch_predictor.py`) still does `unsqueeze(1)` and does not read `header["sequence"]`, so the abstain exists only in the trainer-side `predict_bundle` until that file calls it (exact edit in the R-N/PY2 report). **R-N2 2026-10-05:** `apps/model-inference` now serves a windowed bundle: `torch_predictor` reads `header["sequence"]["window"]`, builds `(1, window, features)` from the last `window` instances (oldest first) and raises `base.Abstain("insufficient_history: have n, need W")` below it; `/predict` answers 422 `abstain:` and `/v2/predict` marks context rows `abstained` and answers on the last row. Tests (`apps/model-inference/tests/test_vol_and_sequence_serving.py`): `a_windowed_bundle_abstains_below_the_window_and_answers_at_it`, `the_input_is_one_window_of_the_latest_bars` (asserts the model sees `(1, 16, 2)`), `without_a_sequence_block_serving_is_unchanged_per_row`, `v2_marks_context_rows_and_abstains_on_short_history`; mutation-checked (removing the length check fails 2). **Still missing:** (1) only the `lstm` architecture is servable here (the zoo's TCN, DLinear, ... live in the trainer's `families/sequence.py`, so serving abstains `sequence_arch_not_served`); (2) the Rust `InferenceGateway` sends one instance per call, never a window of recent bars, so a windowed model called through the platform always abstains today.

`LSTMNet.forward` does `x.unsqueeze(1)` on 2-D rows, so the sequence length is 1 (two copies: `adapters/forecaster.py:18`, `adapters/torch_adapter.py:31`). Rows become windows when the
declaration has `sequence`, the bundle records `sequence.window`, and the serving path builds the same windows.

**Files:** `apps/model-trainer/app/adapters/`, `engine.py` (Set U per-fold hunks stay untouched)
**Acceptance:** inputs are 3-D `(batch, window, features)`; a bundle records `sequence.window = 64`; serving with fewer than 64 bars abstains; a model on windows beats the same model on seq-len 1 on a planted lag-dependent series.
**Traces:** MD-07; FEAT-006 §3, M4

### N-2.6 The sequence zoo: TCN, DLinear, N-HiTS, PatchTST, TFT, TSMixer — L
`PARTIAL` 2026-10-04 (R-N/PY2) — `app/families/sequence.py`: LSTM, TCN (causal), DLinear, compact N-HiTS, PatchTST, TFT (GRN selection + causal attention) and TSMixer, entry points `train_<variant>`; each refuses without `declaration.sequence` or `max_gpu_hours`, stops at the wall-clock budget per epoch (`budget_exhausted`), records `vs_gbm` against a LightGBM on the same windows flattened to lagged features, and sets `promotable` only when that comparison was recorded and the model's test score is strictly lower. Tests (same file, parametrised over 7 variants): `every_variant_trains_on_windows_and_records_vs_gbm`, `wall_clock_budget_stops_training_at_the_epoch_boundary`, `promotable_needs_a_recorded_comparison_that_the_model_wins`, `unpromotable_when_the_gbm_comparison_cannot_be_made`. **Missing:** the reference dataset is a synthetic lag series (no real-data run), point output only (no quantile head), TFT/N-HiTS/PatchTST are compact variants, and a downstream consumer of `promotable` does not exist yet.

SHOULD (ADR-N-02). A sequence model must beat the GBM on lagged features to be used downstream.

**Files:** `apps/model-trainer/app/families/sequence.py` (outside this track)
**Acceptance:** each variant trains on the reference dataset within its `max_gpu_hours`; none is promotable without a recorded comparison against the GBM baseline.
**Traces:** MD-06; FEAT-006 §4

### N-2.7 Probabilistic family: NGBoost, quantile regression forest, conformal wrappers — M
`DONE` *(2026-10-04, PY1)* `families/probabilistic.py`: own Gaussian natural-gradient boosting, quantile regression forest, split conformal. Stated tolerance `COVERAGE_TOLERANCE = 0.05` around 0.90. Evidence: `tests/test_families_probabilistic.py` 5 passed (seeded heteroscedastic data; NGB 0.892, QRF 0.918, conformal 0.898 coverage on held-out rows; NGB sigma tracks the planted heteroscedasticity). Not here: ACI (N-4.1), which builds on `SplitConformal`.

Calibrated intervals; the conformal wrapper is N-4.1's base.

**Files:** `apps/model-trainer/app/families/probabilistic.py` (outside this track)
**Acceptance:** empirical coverage of the 90% interval on held-out folds is within the stated tolerance of 90%.
**Traces:** MD-06; FEAT-006 §4

### N-2.8 Foundation baselines with contamination metadata — M
`PARTIAL` *(2026-10-04, PY1)* `families/foundation.py`. Acceptance clauses met and tested (`tests/test_families_foundation.py` 4 passed): never selectable as candidate (registry refuses, all three), the row carries `contamination {model_cutoff, contaminated, window_start, window_end, note}` naming the window, an absent package gives a typed `unavailable` row with no scores. PARTIAL because none of chronos/timesfm/ttm is installed here: only a Chronos backend is written (untested against the real package) and TimesFM/TTM report `unavailable: adapter not wired`. The model cutoffs in `registry.json` are placeholders to confirm against each model card.

Chronos / TimesFM / TTM zero-shot as **baselines only**, carrying `contamination: {model_cutoff}`; a model whose cutoff is after the window start is flagged.

**Files:** `apps/model-trainer/app/families/foundation.py` (outside this track)
**Acceptance:** a foundation baseline is never selectable as the candidate; its scorecard row names the contamination window.
**Traces:** MD-06 (MAY); FEAT-006 §4

### N-2.9 The bring-your-own-model contract — L
`PARTIAL` 2026-10-04 (R-N/PY2) — `app/byo_interface.py` (`ModelInterface`), `app/byo.py` + `app/byo_runner.py`: a snapshot dir runs in a sandbox subprocess (every `socket.*` event and process creation refused and logged, a model that swallows the error still fails; writes only to scratch; snapshot files read-only; timeout), one fitted model per fold plus a final refit, `feature_order` = `feature_hash` values (or column names flagged `column_name`), bundle records `code_hash` and carries the snapshot, serving re-checks it. Tests (`tests/test_byo.py`, 13): `a_network_call_fails_in_the_sandbox_even_if_the_model_swallows_it`, `bundle_records_the_code_hash`, `a_model_that_reads_a_feature_outside_feature_order_fails_at_predict`, `the_snapshot_is_read_only`, `a_tampered_bundle_snapshot_is_refused_at_serving`; mutations on the audit hook and strict frame fail tests. **Missing:** `tbot.models.ModelInterface` in `apps/tbot`, the Rust `code_handle` / snapshot fetch and a `model_versions` column, serving of `framework=byo` bundles in `apps/model-inference`, and the sandbox is a policy guard, not a hostile-code boundary (ADR-N-PY2-03). **R-N2 2026-10-05:** `apps/tbot/src/tbot/models.py` is the `ModelInterface` the user code imports (`from tbot.models import ModelInterface`), a copy of the trainer's `byo_interface.py`; `tbot[models]` extra declares numpy and pandas. Tests (`apps/tbot/tests/test_models_interface.py`, 4): the class is abstract and each method required, a complete model round-trips save/load, `StrictFrame` names an undeclared feature, and `the_copy_is_identical_to_the_trainers_interface` (mutation-checked: renaming `fit` in the copy fails 2). **Still missing:** the Rust `code_handle` / snapshot fetch and a `model_versions` column, and serving `framework=byo` bundles in `apps/model-inference`.

`tbot.models.ModelInterface` (`fit`, `predict`, `save`, `load`), submitted as a code-snapshot artifact plus a dataset handle, run in the trainer sandbox (no network, read-only snapshot,
scratch space), one fitted model per fold plus the final refit, `feature_order` as DATA-006 `feature_hash` values and the code hash recorded.

**Files:** `apps/model-trainer`, `crates/model-registry`, `apps/tbot` (outside this track)
**Acceptance:** a BYO model with a network call fails in the sandbox; the bundle records the code hash; a model that reads a feature not in `feature_order` fails at predict.
**Traces:** MD-04; FEAT-006 §5, M2

### N-2.10 HPO trials count on a model trial counter — M
`DONE` **R-N 2026-10-04:** migration 0208 (monotonic trigger, SECURITY DEFINER hook, app_role SELECT+EXECUTE only, training_runs insert trigger) applied twice on scratch PG 17; `count_hpo_trials` called after each run, `drive_eval` deflates by the counter. pg tests pass on scratch PG 17 (6/6, `pg_model_platform`); mutation: granting UPDATE to app_role fails `the_counter_cannot_be_lowered...`. Found and fixed: 0044 default privileges give app_role DML on new tables, so 0208 revokes by name. **R-N2 2026-10-05:** the chain that makes the counter move was broken at the trainer: the platform read `metrics["trial_count"]` while the quantile adapters write `metrics["hpo_trials"]`, so no HPO trial ever counted. Now `studio::hpo_trials_reported` reads `hpo_trials`; `hpo.run_hpo` counts every trial it starts (a trial that raised or returned inf still looked at the data). Tests: `apps/model-trainer/tests/test_hpo_trial_count.py` (5: forty trials report forty, a failing trial counts, all-failing counts and tunes nothing, a disabled search counts zero, and a cross-language contract test that reads `studio.rs` for the key and checks all three adapters write it; mutations `range(max_trials - 1)` and renaming the adapter key each fail), model-registry `studio::tests::hpo_trials_are_read_from_the_key_the_trainer_writes`. Counter, monotonic trigger and `app_role` checks are R-N's (`pg_model_platform`). **Not run:** an HPO search end to end through the platform.

Migration 0208: `model_experiments.trial_count` is monotonic and updated only by the job-service counting hook in the same transaction as the job insert; HPO objective CRPS or QLIKE; **every trial counts**.
`app_role` grants in the same migration.

**Files:** `migrations/0208_model_experiments.sql` (new), `crates/model-registry`
**Acceptance:** forty HPO trials raise the counter by exactly forty; a direct UPDATE that lowers it is refused; `app_role` can read and the hook can write (`SET LOCAL ROLE app_role` test).
**Traces:** MD-05; FEAT-006 §9, M3

### ADR rows (PY1)

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-N-PY1-01 | Family baselines live in `metrics["family_baselines"]` (a list with aligned `series`); the legacy `metrics["baselines"]` `{naive, zero_shot}` dict is untouched. | Overwrite `baselines` with the list. | scoring.py already writes a dict there and the report reads it; the Rust side agreed to the new key. |
| ADR-N-PY1-02 | A baseline missing a required input (HARQ without realised quarticity) is a `refused` row in an evaluation and a typed `MissingRequirement` (`data_error`) when trained directly. | Drop the row, or fail the whole evaluation. | A dropped row reads as a pass (ADR-N-05); failing everything blocks evaluation on one optional comparator. The refused row is NotComputable downstream. |
| ADR-N-PY1-03 | Vol models serve only the conditional sigma, filtered through the recent bars from the model's own initial variance, and abstain with fewer than `min_bars` (default 30) bars. Legacy GARCH bundles are refused. | Keep a sigma_uncond fallback for short windows, or refit on the window. | The fallback is the bug N-4.2 removes; refitting at serve time is a training step in the serving path. |
| ADR-N-PY1-04 | Jump-model labels are the argmin of the causal forward DP value; the continuous JM is a softmax over those values (online approximation), not the offline continuous-JM optimum. | Offline Viterbi or convex continuous-JM labels. | Offline labels use the future, which N-2.4 forbids; the offline solution is used only to fit centroids on the training window. |
| ADR-N-PY1-05 | Own numpy/scipy implementations of GARCH-family MLE, HMM EM and natural-gradient boosting. | Add `arch`, `hmmlearn`, `ngboost` dependencies. | They are not installed here and not in pyproject; the models are small and fully tested. |
| ADR-N-PY1-06 | A foundation baseline with no installed package is a typed `unavailable` row; an unknown evaluation window start is flagged contaminated. | Skip the row silently, or assume uncontaminated. | Absence and unknown provenance must be visible, not passed. |
| ADR-N-PY1-07 | The platform's top-level `declaration` and `family` request fields are merged into `definition` at the start of `run_training` / `run_evaluation`. | Thread them as separate arguments. | Registry lookup and sequence windowing read one place; ModelDefinition cannot carry free keys on the Rust side. |

### ADR rows (PY2)

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-N-PY2-01 | Windows are cut inside each split (train, validation, test separately), so each later split loses its first W-1 rows; `stride` thins training windows only and validation/test use every window. | Let a test window reach back into train rows for context; stride on all splits. | "A window never crosses a fold boundary" is checkable only if the rule is absolute; a score must not be a function of the stride. |
| ADR-N-PY2-02 | The bundle header carries a top-level `sequence {window, stride}` (a new optional `wrap_bundle(sequence=...)` argument), and serving abstains with a typed `insufficient_history` rather than padding. | Put it in `header.extra`; left-pad short histories with zeros or repeats. | The inference reader already reads top-level keys; padding gives a confident answer about bars that were never seen. |
| ADR-N-PY2-03 | The BYO sandbox is a subprocess policy guard (audit hook on socket/process/write events, read-only snapshot, scratch dir, timeout, stripped environment, violation log the parent reads). | A container/network-namespace sandbox now; trusting the exception the model raises. | No container runtime is guaranteed in the trainer image; `ctypes` can still bypass a hook, so hostile-code isolation remains the deployment's network namespace. The violation log means a model that catches the error still fails. |
| ADR-N-PY2-04 | CUSUM and strategy-event sampling are applied trainer-side (`app/sampling.py`, in `run_training`), and fold ranges are remapped onto the thinned frame. | Implement in `crates/dataplane` (outside this track's territory). | Reachable today; the Rust lowering can keep naming them `Unsupported` until it samples natively. |
| ADR-N-PY2-05 | A strategy entry timestamp maps to the last bar at or before it. | Round to the nearest bar, or the first bar after. | Nearest/next can land on a later bar, which uses information the entry did not have. |
| ADR-N-PY2-06 | `h = "<k>sigma"` uses a causal expanding sigma (no event before 20 returns). | Sigma of the whole series. | A later volatility regime would change an earlier event; tested by truncation. |
| ADR-N-PY2-07 | `promotable` for a sequence model is true only if a GBM-on-lagged-windows comparison was recorded on the test windows and the model's score is strictly lower; a failed comparison is recorded as unrecorded, never as a pass. | Promote on absolute metrics; treat a missing comparison as pass with a warning (ADR-N-05). | FEAT-006 section 4: a sequence model must beat the GBM to be used downstream. |
| ADR-N-PY2-08 | The sequence zoo requires `max_gpu_hours` (no default) and stops gracefully at the epoch boundary, recording `budget_exhausted`, instead of raising `BudgetExceeded`. | Default budget; raise on overrun. | AT-66: no platform default. A stopped-at-budget model is a result with a flag, not a lost run; the promotion rule still judges it. |
| ADR-N-PY2-09 | Leakage check (b) treats a training row as leaking when its label span intersects the test fold's span (earliest label_start to latest label_end) or it lies in the embargo rows after the test fold; the trainer's run reports the feature-truncation check as `skipped`. | Overlap with only the test rows' own spans; mark truncation as passed. | Spans of a contiguous fold are one interval; the trainer has no feature expressions, and an unrun check must not read as a pass. |
| ADR-N-PY2-10 | `aci_cusum` uses the platform monitor's k = 0.05, h = 4.0 and restart-to-zero, resets `alpha_t` to the target and cuts the score window to the excursion start (at least `min_window` = 20 recent scores). | Clear the window entirely; own constants. | The alert and the reset must agree; a window of a few scores yields a noisy quantile. Detection of over-coverage (a volatility fall) is slower than of misses (about 60-80 bars versus under 30). |
| ADR-N-PY2-11 | Transforms (winsorize, scaler, lags) were not implemented in the trainer. | Apply them inside `engine.prepare`. | They must be fitted per fold and the engine's per-fold hunks belong to another session; applying them wrongly would be silent leakage. |
