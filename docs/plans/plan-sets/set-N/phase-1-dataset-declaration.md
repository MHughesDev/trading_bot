# Phase 1 — The dataset declaration and `dataset_build`

**Completion: 71% (5 / 7 tasks), plus 2 `PARTIAL`.**

**Requirement IDs:** MD-01, MD-02, MD-03
**Spec:** [FEAT-006](../../../specs/FEAT-006-model-platform-v2.md) §3
**Depends on:** Phase 0.
**Blocks:** Phases 2 and 3.
**Migration range:** 0207.

A training run declares its data; it does not describe it in prose. The declaration is a typed, validated, canonically hashed object, and purge and embargo are
*derived* from the label horizon, never hand-set (INV-12, INV-15, MD-02). The pure half (type, validation, canonical hash, derived embargo, lowering) is built in
`crates/model-registry/src/dataset_declaration.rs`; the build job and leakage harness are the production half.

---

### N-1.1 `DatasetDeclaration`: the type, validation and canonical hash — M
`DONE` 2026-10-04 — `dataset_declaration::{DatasetDeclaration, Validated}`: typed, `deny_unknown_fields`, `weights` required, canonical `declaration_hash`, window clipped to a cutoff with the clip reported. Tests (model-registry lib): `no_field_can_set_purge_or_embargo`, `weights_are_required_and_never_defaulted`, `the_hash_ignores_instrument_order_and_follows_every_field`, `each_validity_rule_names_its_field`. **Missing:** nothing calls it; no route accepts a declaration. **R-N 2026-10-04:** reached: `studio::resolve` (called by `ModelManager::start_train`) and `POST /api/models/declarations/validate`; model-registry lib tests 144 pass incl. `studio::tests`. `cargo check -p api --lib` and `-p model-registry --all-targets` green.

Universe, timeframe, window, features, transforms (winsorize, scaler, lags), sampling (`every_bar | cusum | strategy_events`), target (`forward_return | realized_vol |
triple_barrier | meta_label | rank | direction` with horizon and params), weights (`uniqueness | time_decay | none`), optional sequence window and stride, and the cv scheme.
Canonical JSON (keys sorted) gives `declaration_hash`; the resolved `dataset_hash` still comes from the existing planner.

**Files:** `crates/model-registry/src/dataset_declaration.rs` (new), `crates/model-registry/src/lib.rs` (one `pub mod` line)
**Acceptance:** an unknown field is refused; a declaration with no `weights` is refused (no silent `none`, ADR-P2-22); two declarations equal up to key order hash equal and any field change changes the hash; a window ending after the project cutoff is clipped and the clip is reported; a sequence window larger than the training span is refused.
**Traces:** MD-01; FEAT-006 §3; ADR-P2-22

### N-1.2 Derived purge and embargo — S
`DONE` 2026-10-04 — `Derived` in `dataset_declaration`: purge is the label horizon; embargo is `max(dataplane::compute_embargo, 1% of the sample)`, with a 24-hour floor at 5m or finer (ADR-N-03). Tests: `embargo_is_derived_on_a_table_of_cases`, `the_fine_resolution_floor_is_24_hours_of_bars`, `a_good_declaration_validates_and_derives`; the type has no purge or embargo field. The look-back is a conservative estimate (largest integer argument in a feature expression, lags, sequence window). **Missing:** a caller; `purged_kfold` purge shorter than the horizon cannot be declared at all, which makes the planned refusal unnecessary rather than tested. **R-N 2026-10-04:** derived embargo is what `lower_for_trainer` hands the planner (`studio::tests::a_declared_run_lowers_with_its_derived_embargo...`).

Purge is the label horizon; embargo is `max(label horizon, 1% of sample)` and for datasets at or below 5m a documented floor (FEAT-006 open question 3,
ADR-N-03: 24 hours of bars). Uses `dataplane::split` where it already computes this; the declaration has no field to set either value.

**Files:** `crates/model-registry/src/dataset_declaration.rs`
**Acceptance:** no field of the declaration can set purge or embargo (`deny_unknown_fields` test); the derived embargo equals `max(horizon, ceil(1% n))` on a table of cases; a 5m dataset gets the floor; a `purged_kfold` declaration whose purge is shorter than its label horizon is refused.
**Traces:** MD-02; FEAT-006 §3; INV-12, INV-15

### N-1.3 Lowering to a `DatasetRequest` — M
`DONE` 2026-10-04 — `Validated::lower` builds a `DatasetRequest` (content-keyed `LabelSpec` and `SplitSpec`, embargo raised to the derived figure) for the supported fields and returns **every** unsupported one by name: named universes, free-form feature expressions (only `set:<ref>` lowers), cusum and strategy-event sampling, sequence windows, transforms, and high/low triple barriers until N-1.4. Tests: `a_supported_declaration_lowers_with_its_derived_embargo`, `unsupported_fields_are_named_not_dropped`, `the_cv_scheme_maps_to_the_split_kind`. **Missing:** a caller, and `DatasetManager::plan` has not been run on a lowered request. **R-N 2026-10-04:** `Validated::lower_for_trainer` + `TrainerExecutes`; `start_train` plans exactly the lowered request. `DatasetManager::plan` on a lowered request is exercised only by the unit tests, not against a live store. **R-N2 2026-10-05:** lowering now sets `LabelSpec.barrier_path` for triple barriers and `LabelSpec.sampling` for CUSUM, so neither is `Unsupported` any more (strategy-event sampling still is, in `lower()`; `lower_for_trainer` hands it to the trainer). `unsupported_fields_are_named_not_dropped` was rewritten around that, and `studio::tests::a_cusum_declaration_samples_in_the_build_and_the_trainer_is_not_told_to_again` shows the trainer's declaration carries `every_bar` when the build already sampled (otherwise the trainer would thin the rows twice).

`DatasetDeclaration -> DatasetRequest` for the features the existing planner supports, and a typed `Unsupported { field, why }` for the ones it does not yet
(cusum and strategy-event sampling, sequence windows) rather than a silent drop.

**Files:** `crates/model-registry/src/dataset_declaration.rs`, `crates/model-registry/src/datasets.rs` (additive)
**Acceptance:** a declaration using only supported fields lowers and plans to a `dataset_id`; each unsupported field returns `Unsupported` naming the field; the lowering is deterministic.
**Traces:** MD-01; FEAT-006 §3

### N-1.4 Triple-barrier labels on the intrabar high/low path — M
`DONE` 2026-10-04 (R-N/PY2) — `labelers.triple_barrier_labels(path="high_low")`: the bar's high/low touches a barrier, a bar spanning both takes the stop, a missing high/low column is a `DataError`. Tests (`tests/test_labelers_high_low.py`, 8): `a_bar_spanning_both_barriers_labels_as_the_stop`, `a_wick_only_through_the_target_labels_as_the_target`, `close_path_result_is_unchanged_on_a_series_with_no_wicks[devol True/False]` (mutation-checked: flipping the tie-break fails the spanning test). **Missing:** nothing calls `labelers.py` in production (labels reach the trainer already materialised), and `crates/dataplane/src/label.rs` has no barrier labelling at all (only intervals and uniqueness weights), so the Rust dataset build still cannot produce high/low labels. **R-N2 2026-10-05:** Rust half built. `crates/dataplane/src/barrier.rs`: `label_event` (high/low or close path, a bar spanning both barriers is the stop, an unfinished vertical barrier is `None`, not a timeout), `causal_sigma`. Wired: `LabelSpec.barrier_path` (omitted from the serialised form when absent so every existing `label_spec_id` is unchanged) is set by the lowering and read by `model_registry::dataset_labels::apply`, which `DatasetManager::build_frame` calls on the aligned frame. Tests: dataplane lib `barrier::tests` (6: `a_bar_spanning_both_barriers_labels_as_the_stop`, `a_wick_only_through_the_target_labels_as_the_target`, `the_close_path_equals_the_high_low_path_on_a_series_with_no_wicks`, `a_label_ignores_bars_past_its_vertical_barrier`, `sigma_at_a_bar_never_reads_a_later_bar`, `a_timeout_and_an_unfinished_label_are_distinct`), model-registry `dataset_labels::tests` (6, incl. `the_high_low_path_sees_a_wick_the_close_path_does_not`). Mutation-checked: letting the target win a both-barrier bar fails the spanning test; dropping the CUSUM row filter fails `cusum_keeps_only_event_rows...`. **Not run:** `build_frame` against a live ClickHouse store (the frame-level tests use synthetic bars).

`labelers.py` triple-barriers on the close path only. Add the `high_low` path: a barrier is touched when the bar's high or low crosses it, and when both barriers
are inside one bar the stop is taken first (the pessimistic reading, as in FEAT-005 §3.4).

**Files:** `apps/model-trainer/app/labelers.py` (outside this track), `crates/dataplane/src/label.rs`
**Acceptance:** a bar whose range spans both barriers labels as the stop; a bar that only wicks through the target labels as the target; the close-path result is unchanged for a series with no wicks.
**Traces:** MD-03; FEAT-006 §3

### N-1.5 The `dataset_build` job and the leakage harness — L
`DONE` 2026-10-04 (R-N/PY2) — Python half: `app/leakage.py` (`label_fold_overlap_check`, `feature_truncation_check`, `build_report` manifest `{engine_version, data_snapshot, checks}`, typed `LeakageDetected` with terminal `leakage_detected`) hooked into `worker.run_training` before any fit; metrics carry `leakage` (`checked:false` + reason for a legacy frame). Tests (`tests/test_leakage.py`, 16): overlapping pair fails and names rows, embargo, causal vs centred vs whole-series-scaled features, `run_training_refuses_a_leaking_dataset_before_training`, `run_training_on_a_legacy_dataset_records_unchecked`; four mutations each fail tests. **Missing:** the `dataset_build` job kind, Parquet writer and registration refusal (`crates/jobs`, `migrations/0207`), and a caller of `feature_truncation_check` (the trainer holds no feature expressions; its run reports `skipped`). **R-N 2026-10-04:** Rust half built: `leakage.rs` (truncation self-test, label/fold overlap on `SplitSpec::expand`, plus `features::leakage` checks) runs in `DatasetManager::materialize` before anything is stored; migration 0207 CHECK refuses a failing report. Lib tests pass; pg CHECK probe passes. No `dataset_build` job kind (crates/jobs outside territory). **R-N2 2026-10-05:** the job kind and its worker. `JobKind::DatasetBuild` already existed with no worker (a submitted build failed `no_worker`). New: `crates/jobs/src/kinds/dataset_build.rs` (manifest, `DatasetBuilder` trait, `run_build`: a leak is terminal `leakage_detected` and not retried, an unusable request is terminal `integrity_rejected`, an outage is retried), `crates/api/src/dataset_build_worker.rs` (the builder over `DatasetManager::plan` + `materialize`; a typed `datasets::LeakageRefused` replaces the old `ensure!` so the job can tell a leak from an outage), registered on the Research class in `apps/platform/src/main.rs`. Reachable by `tbot jobs submit dataset_build '{...}'` through the generic `/api/jobs`. Tests: jobs lib `kinds::dataset_build::tests` (4, mutation-checked: mapping a leak to `data_error` fails), api lib `dataset_build_worker::tests` (2). The leakage checks themselves (overlap, truncation self-test, manifest with engine version and data snapshot) are R-N's `leakage.rs`, run inside `materialize` before anything is stored. **Not run:** a build through a live job store and ClickHouse; the manifest carries a lowered `DatasetRequest` (nothing yet turns a declaration into the manifest).

A job kind that computes features (DATA-006 batch), builds labels, writes Parquet (features, label, `label_start`, `label_end`, weight, fold ids, sequence index), runs the
leakage harness (feature truncation self-test plus label/fold overlap) and records a manifest.

**Files:** `crates/jobs` (kind), `crates/model-registry`, `apps/model-trainer`, `migrations/0207_dataset_declaration.sql` (new: `dataset_versions.dataset_spec`, `dataset_handle`, `app_role` grants)
**Acceptance:** a dataset with an overlapping label/fold pair fails the harness and is not registered; a feature that changes when the series is truncated fails the self-test; the manifest records the engine version and data snapshot.
**Traces:** MD-01; FEAT-006 §3

### N-1.6 Specification counting — M
`PARTIAL` 2026-10-04 — `dataset_declaration::specification_delta`: additions counted, removals never subtracted (`specification_count_rises_on_additions_and_never_falls_on_removals`). **Missing:** the hook that adds it to the Experiment's count in `crates/ledger`. **R-N 2026-10-04:** migration 0221 + `studio::record_specifications` called from `start_train` when `TrainRequest.experiment_id` is set; pg test `specifications_only_rise_and_an_unknown_experiment_is_refused` passes on scratch PG 17 (verified 2026-10-04).

Every feature, transform or interaction added relative to the parent dataset increments the owning Experiment's specification count (BACKTEST_SUITE v2 §3).

**Files:** `crates/model-registry`, `crates/ledger` (outside this track)
**Acceptance:** adding two features to a child declaration raises the count by two; removing a feature never lowers it.
**Traces:** EV-03; FEAT-006 §3

### N-1.7 Sampling modes: CUSUM and strategy events — L
`PARTIAL` 2026-10-04 (R-N/PY2) — trainer-side (ADR-N-PY2-04): `app/sampling.py` `cusum_events` (symmetric, `h` absolute or `"<k>sigma"` with a causal expanding sigma), `strategy_event_positions` (last bar at-or-before each entry, never later), `apply_to_request` subsamples the frame and remaps fold ranges, wired into `worker.run_training`; `label_start`/`label_end` kept. Tests (`tests/test_sampling.py`, 14 + `test_leakage.py::run_training_subsamples_when_the_declaration_says_cusum`): events land exactly on the planted jump bars, a later volatility regime cannot erase an earlier event (mutation-checked), timestamps mapped without a later bar. **Missing:** `crates/dataplane` does not sample (lowering still returns `Unsupported` for cusum/strategy events), and the platform must resolve `strategy_version` to `entry_timestamps` before the call. **R-N2 2026-10-05:** `crates/dataplane/src/sampling.rs`: `cusum_events` (threshold from the sigma known before the bar, so a jump cannot widen its own threshold) and `strategy_event_indices` (the last bar at or before each entry, never a later one). CUSUM is wired into the dataset build (`LabelSpec.sampling`, `dataset_labels::apply`); tests `cusum_events_land_exactly_on_the_planted_jump_bars`, `a_later_regime_cannot_erase_an_earlier_event` (mutation-checked: using the bar's own sigma fails it), `strategy_events_map_to_the_last_bar_at_or_before_the_entry`. **Still missing:** `strategy_event_indices` has no Rust caller: resolving a `strategy_version` to its entry timestamps happens outside the build, so strategy-event sampling stays in the trainer (ADR-N-PY2-04) and `lower()` still names it `Unsupported`.

`cusum(h)` event sampling and `strategy_events(strategy_version)` (needed by the meta-labeller, N-4.4).

**Files:** `crates/dataplane`
**Acceptance:** CUSUM events on a planted jump series land on the jump bars; strategy-event sampling uses only the strategy's entry timestamps and never a later bar.
**Traces:** MD-01; FEAT-006 §3
