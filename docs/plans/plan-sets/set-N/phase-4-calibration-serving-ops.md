# Phase 4 — Calibration, serving and operations

**Completion: 67% (4 / 6 tasks), plus 2 `NOT STARTED`.**

**Requirement IDs:** MD-11, MD-12, MD-13
**Spec:** [FEAT-006](../../../specs/FEAT-006-model-platform-v2.md) §6, §8, §9
**Depends on:** Phases 2 and 3.
**Migration range:** 0209.

Adaptive conformal calibration, honest GARCH serving, human-approved alias promotion, the meta-labeller, and the Set H operational items that survived re-derivation.

---

### N-4.1 ACI calibration as a wrapper model version — L
`DONE` 2026-10-04 (R-N/PY2) — `app/calibration.py`: `split`, `aci` (Gibbs-Candes) and `aci_cusum` (CUSUM on the miss stream with the monitor's k = 0.05, h = 4.0, restart to zero; a fire resets `alpha_t` and cuts the score window to the change point), point or CQR-band inputs, `spread_90`, rolling coverage, `reset_indices`; `POST /calibrate` in `app/main.py`. Tests (`tests/test_calibration.py`, 10): `rolling_coverage_stays_closer_to_nominal_than_static_split_after_a_regime_shift` (gap < 0.5x split), `a_reset_fires_within_the_stated_lag_of_the_shift_and_not_before` (lag <= 30 bars for a x3 volatility rise; a x3 fall resets within 120), `a_reset_restores_alpha_t_and_cuts_the_window_to_the_change_point`; four mutations each fail tests. **Missing:** no Rust caller of `/calibrate`, `tbot models calibrate`, the wrapper model version, and the `quality_monitor.rs` hook. **R-N 2026-10-04:** Rust: `calibration::CoverageCusum` (5 tests) in `quality_monitor`, `POST .../calibrate` -> trainer `/calibrate` -> wrapper version row; migration 0222 creates the tables the monitor never had. Missing: `QualityMonitor::spawn` is never called (apps/platform main.rs), realised values are never filled into `model_forecasts`, serving does not consume `spread_90`. **R-N2 2026-10-05:** the four gaps R-N named are closed. (1) Realised outcomes: `forecast_realisation::realise_pending` joins `model_forecasts` to the bars that realise them (entry = last bar closed by the forecast time, exit = first bar closing at or after the horizon, within one bar of it); it runs at the start of every `QualityMonitor` cycle (the monitor is spawned, `main.rs:598`); migration `0227_forecast_realisation.sql` adds `timeframe` and `log_forecast` now writes `horizon_bars` and `timeframe` (from the version's `config_json`, which now records them). A legacy row with neither is counted `unresolvable` and never guessed. (2) Serving: `ForecastResult.calibrated_spread_90` is the latest finite `spread_90` of the newest calibration wrapper of that version and `forecast_to_output` prefers it over `q95 - q05`. (3) `tbot models calibrate <model> <version> --method aci|aci_cusum`. Tests: model-registry lib `forecast_realisation::tests` (4) and `inference_gateway::tests::serving_consumes_the_calibrated_spread_over_the_raw_quantile_width`; pg (scratch PG 17) `pg_forecast_realisation` (3: a due forecast realised from fake bars to the exact return, a pending and a legacy row left alone, no bar store means every row waits, `app_role` can write `realized` and `timeframe`) and `pg_model_ops::the_calibrated_spread_comes_from_the_newest_wrapper_of_that_version`; tbot `test_models_calibrate` (2). Mutation-checked: an entry bar one period late fails 2 realisation tests. **Not run:** realisation against a live ClickHouse store (the pg test supplies bars through the `ClosedBars` trait).

`tbot models calibrate --method aci|aci_cusum`: adaptive conformal inference with CUSUM change-point resets hooked into `quality_monitor.rs`; reports coverage through time; sizing consumes the calibrated `spread_90`.

**Files:** `crates/model-registry/src/quality_monitor.rs`, `apps/model-trainer`
**Acceptance:** on a synthetic regime-shift series the wrapper's rolling coverage stays closer to nominal than static split conformal; a reset fires within the stated lag of the shift.
**Traces:** MD-11; FEAT-006 §8, M7

### N-4.2 GARCH serving returns the conditional sigma and never falls back — M
`DONE` *(2026-10-04, PY1)* Trainer side done: `families/vol.py::conditional_sigma` / `serve_from_state` return the conditional one-step sigma from recent bars or raise `Abstain(reason)`; the unconditional value is a fit-time diagnostic that no serving function reads; the trainer eval path (`worker._eval_vol`) uses the per-row conditional sigma and the old unconditional tile is gone; legacy bundles are refused (`state_from_bundle_inner`). Evidence: `tests/test_families_vol.py` (spike gives sigma above unconditional; no bars, too few or NaN abstain with a reason; grep-style test that no serving source mentions `sigma_uncond`; mutation returning the unconditional value fails). PARTIAL: `apps/model-inference/app/predictors/garch_predictor.py` (outside this track) still serves `sigma_uncond` and cannot read the new bundle; the exact edit is in the PY1 report. **R-N2 2026-10-05:** serving is wired. `apps/model-inference/app/predictors/garch_predictor.py` filters the variance recursion (garch, gjr, egarch, realised garch; mirrors the trainer's `vol.py`) through `series["recent_returns"]` and returns the conditional sigma, or raises `Abstain(reason)` for missing, too few or non-finite bars and for a legacy bundle; `/predict` answers 422 `abstain:` (not counted by the sidecar client's circuit breaker) and `/v2/predict` an `abstained` answer with the reason. The Rust gateway reads the version's recent bars through the point-in-time reader (`recent_bars::RecentBars`) and sends them; an abstain is traced `abstain` and does not fall back to another alias. Tests (`test_vol_and_sequence_serving.py`, 12 incl. sequence): `serving_after_a_volatility_spike_is_above_the_calm_sigma_and_matches_the_recursion` (independent recomputation), `missing_short_or_nonfinite_bars_abstain_with_a_reason` (3), `a_legacy_bundle_with_only_an_unconditional_sigma_abstains`, `the_unconditional_sigma_is_never_served` (greps the serving source), all four kinds serve; mutation: returning `sigma_uncond` fails 4. Rust: `recent_bars::tests` (3). **Not run:** gateway to sidecar to ClickHouse end to end.

The inference sidecar loads the recent bars window from ClickHouse (or the stream). The unconditional fallback is removed; missing data is an error with an abstain.

**Files:** `apps/model-inference` (outside this track), `crates/model-registry/src/sidecar.rs` (Set U hunks stay)
**Acceptance:** serving after a volatility spike returns a sigma above the unconditional one; with no bars it abstains with a reason; there is no code path that returns the unconditional value.
**Traces:** MD-12; FEAT-006 §8, M8

### N-4.3 Alias promotion is a human decision — M
`DONE` **R-N2 2026-10-05:** R-N built the proposal and decision routes (`require_human` on promote, rollback and decide; `propose_promotion` for anyone who can see the model; pg `a_promotion_proposal_is_pending_until_a_person_decides_and_app_role_can_use_the_table`). Here the agent tool `promote_model_version` was still posting to `/promote` and getting a 403; it now files `POST /api/models/{id}/versions/{v}/promotion-proposals` with a required rationale (descriptor says PROPOSE, within the 1,200-byte budget and the 16-per-namespace cap). Tests: mcp-server-lib `tools::models::promotion_tests` (3; mutation: pointing it back at `/promote` fails), api lib `models_studio::human_gate_tests` (2: a service token is refused with a 403, and every handler in `models.rs` that moves an alias calls `require_human` before it does), pg proposal test above.

`@production` promotion is an approvals-inbox action; agents can only propose it (AGENT-001 §16, AGENT-013 ring 1). **Check first** what the `Proposal` path already offers (Set T T-1.x).

**Files:** `crates/api` approvals, `crates/mcp-server/src/tools/models.rs` (outside this track)
**Acceptance:** an agent-proposed promotion waits for a human; the tool cannot move an alias on its own (a token-scope test).
**Traces:** MD-13; FEAT-006 §8, M9

### N-4.4 The meta-labeller — M
`NOT STARTED`

`tbot models meta-labeller <strategy_version> --experiment <id>`: builds a `strategy_events` declaration with a `meta_label` target, trains through walk-forward, produces a prediction series and proposes an SLv2 move.

**Files:** `apps/tbot`, `crates/mcp-server/src/tools` (outside this track)
**Acceptance:** a meta-labelled strategy completes the funnel on a fixture; the proposed move is `add_filter` or `change_sizing` and is not applied.
**Traces:** FEAT-006 §6

### N-4.5 Set H operational items: used-by, traces and rollups, rate limits, nightly retrain — M
`DONE` **R-N2 2026-10-05:** each of the four has a test that fails when it is removed (verdicts and the two dropped items are in N-0.2 and ADR-N-R2-05). Pg on scratch PG 17, `crates/model-registry/tests/pg_model_ops.rs` (5): used-by lists only strategies referencing the model; the trace rollup counts outcomes and latency percentiles; a model past its rate limit is refused and traced `rate_limited` through `InferenceGateway::forecast`; only an `auto_retrain` model with no run in flight is due (`RetrainScheduler::due_models`); the calibrated spread. Lib: `rate_limit::tests` (4, mutation-checked: an `admit` that never refuses fails 3). New code: `rate_limit.rs`, `InferenceGateway::{admit, with_rate_limit, trace_rollup}`, `RetrainScheduler::due_models`. **Not done:** scheduling evaluation after an auto-retrain (N-4.5b in N-0.2).

H-4.5 `GET /api/models/:id/used-by` exists (verify), H-4.6 inference traces and cost/latency rollups, H-4.7 inference rate limits and permissions, H-4.8 nightly retrain (`scheduler.rs` exists, verify).
Each gets a verdict in N-0.2 and this task closes the ones that remain.

**Files:** `crates/model-registry`, `crates/api` (outside this track)
**Acceptance:** each of the four has a test that fails when it is removed.
**Traces:** Set H 4.5–4.8

### N-4.6 Model storage columns and the series link tables — S
`NOT STARTED`

Migration 0209: `model_versions.family`, `code_handle`, `engine_version`, `requires_retrain`, `contamination`; `model_series_links`. Not the prediction-series table (Set U built it).

**Files:** `migrations/0209_model_platform_v2.sql` (new)
**Acceptance:** idempotent `ADD COLUMN IF NOT EXISTS`; `app_role` grants; an `app_role` test.
**Traces:** FEAT-006 §9

### ADR rows (R-N2)

| ID | Decision | Rejected alternative | Why |
|---|---|---|---|
| ADR-N-R2-01 | A GARCH-family model is served from the recent returns the **platform** supplies as `series["recent_returns"]` (the gateway reads them through the point-in-time bar reader); the sidecar holds no data connection. | The sidecar queries ClickHouse itself. | It would need its own PIT reader, restatement handling and credentials; two readers of the same bars is how a serve and a train disagree. |
| ADR-N-R2-02 | The serving recursion is duplicated into `apps/model-inference` (as `bundle.py` already is), not imported from the trainer. | Share one module. | They are separate images; a shared import couples deploys. A test recomputes the recursion independently so a drift in the copy is caught. |
| ADR-N-R2-03 | An abstain is HTTP 422 with `abstain: <reason>`; the Rust client does not count it as a sidecar failure and the gateway returns `None` without trying the fallback alias. | Return a flat forecast; let the circuit breaker count it. | A flat forecast is the silent fallback N-4.2 removes; counting a refusal as an outage opens the breaker after five abstains and blinds every model on the sidecar. |
| ADR-N-R2-04 | A forecast's realised value is the simple return from the last bar closed at forecast time to the first bar closing at or after the horizon, and only if that bar is within one period of it; otherwise it waits. A row with no recorded horizon or timeframe is counted and never realised. | Nearest bar to the horizon; infer the horizon from the model's definition. | A nearest bar can be before the horizon (a look at the future's past, or a shorter horizon than claimed); the definition can have changed since the forecast was served. |
| ADR-N-R2-05 | H-4.6's ClickHouse trace row and cost field are dropped; the Postgres `inference_trace` events and the rollup stand. | Build a ClickHouse trace table and a cost estimate. | There is no cost data for local inference and no consumer of a second trace store; the Postgres rollup answers every question asked of it. A cost can be added when a hosted model needs one. |
| ADR-N-R2-06 | The Gate 3 model binding is a trait in `crates/backtest` implemented in `crates/api`, and the gate worker takes the bindings from its manifest. | `crates/backtest` depends on `crates/model-registry`; resolve the bindings inside the gate worker from the strategy. | The dependency already points the other way (cycle); resolving a strategy's model slots is Set U's, and the manifest field is the seam it will fill. |
| ADR-N-R2-07 | CUSUM sampling is applied by the dataset build and the trainer is told `every_bar`; strategy events stay in the trainer. | Apply it in both; build strategy-event sampling in Rust now. | Applying CUSUM twice thins the sample twice; strategy events need entry timestamps the build cannot resolve. |
| ADR-N-R2-08 | `LabelSpec` gains `barrier_path` and `sampling`, both omitted from the serialised form when absent. | A new spec type, or always serialise the fields. | Every existing `label_spec_id` and `dataset_id` hashes the serialised spec; new fields must not change a hash that did not use them. |
| ADR-N-R2-09 | Barrier widths and the CUSUM threshold use a 50-bar causal rolling standard deviation of 1-bar returns. | A whole-series or EWMA sigma. | A whole-series sigma uses the future; 50 bars is long enough to be stable at 1-minute resolution and short enough to follow a regime. The window is one constant (`SIGMA_WINDOW`). |
| ADR-N-R2-10 | The inference rate limit counts only calls that reach the sidecar (prediction-cache hits are free), per model, default 1200 per minute. | Count every call; a per-user limit. | The limit protects the sidecar; a cache hit costs it nothing. A per-user limit does not stop one runaway strategy that every user shares. |
