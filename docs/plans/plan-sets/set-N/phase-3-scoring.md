# Phase 3 — Scoring additions and the hard usage rules

**Completion: 0% (0 / 6 tasks), plus 5 `PARTIAL`, 1 `NOT STARTED`.**

**Requirement IDs:** MD-09, MD-10
**Spec:** [FEAT-006](../../../specs/FEAT-006-model-platform-v2.md) §7
**Depends on:** Phase 2 (N-2.2 for the HARQ baseline).
**Migration range:** none.

An honest score is the whole point of the platform's model side. The additions in FEAT-006 §7 are pure statistics, and the *usage rules* are what turn them from a
report into a constraint: a return forecast without a significant out-of-sample R-squared may not size a position, a vol forecast that loses to HARQ may not size one, and nothing sizes on VaR or ES
without a passed ES backtest.

**Built by this set on 2026-10-04:** `crates/model-registry/src/scoring_v2.rs` (pure functions and the usage-rule verdict). No production caller yet: N-3.6 wires the verdict into the
validator and the gate.

---

### N-3.1 The scoring additions — L
`PARTIAL` 2026-10-04 — `scoring_v2`: `oos_r2` (Campbell-Thompson), `clark_west` (Newey-West), `qlike` and `variance_mse`, `mincer_zarnowitz` (exact chi-square-2 Wald p), `es_backtest_gaussian` (Acerbi-Szekely Z1, Z2, Monte-Carlo p), `fissler_ziegel` (FZ0), `utility_gain`. Every function returns a typed `ScoreError`, never NaN. Tests: `clark_west_has_power_against_real_predictability_and_size_under_noise` (60 seeds each way), `qlike_is_minimised_by_the_mean_variance_and_is_zero_when_exact`, `a_calibrated_gaussian_es_forecast_passes_and_an_understated_tail_fails`, `the_fissler_ziegel_loss_is_minimised_by_the_true_var_es_pair`. **Missing:** purged MDA (N-3.3); the ES backtest covers a Gaussian predictive only; no caller. **R-N 2026-10-04:** now reached through `verdict::judge_metrics` (oos_r2, clark_west, qlike, es_backtest_gaussian).

Campbell-Thompson out-of-sample R-squared against the historical mean; Clark-West for nested models; QLIKE and MSE on variance; Mincer-Zarnowitz regression with recalibration;
Acerbi-Szekely ES backtests (Z1 and Z2); the Fissler-Ziegel joint VaR-ES loss; the mean-variance utility gain. Diebold-Mariano already exists in `scoring.py`.

**Files:** `crates/model-registry/src/scoring_v2.rs` (new), `crates/model-registry/src/lib.rs` (one `pub mod` line)
**Acceptance:** each statistic matches a hand-computed value on a small fixture; Clark-West rejects on a series with planted predictability and does not on noise (rate checked over seeds); QLIKE is minimised by the true variance; the ES backtest rejects a forecast that understates the tail and accepts a calibrated one; every function returns a typed error, never NaN, on too-short input.
**Traces:** MD-09; FEAT-006 §7; BACKTEST_SUITE v2 T4, T7, T20

### N-3.2 The hard usage rules as a verdict type — M
`PARTIAL` 2026-10-04 — `scoring_v2::{return_forecast_rule, vol_forecast_rule, tail_risk_rule, judge_binding, BindingVerdict}`; `BindingVerdict` has private fields and one constructor, `NotComputable` blocks, and every boundary is strict. Tests: `the_return_rule_is_strict_at_both_boundaries`, `the_vol_rule_needs_a_strict_win_over_harq_and_a_missing_score_blocks`, `a_binding_is_allowed_only_if_every_rule_in_play_passed` (FEAT-006 M6 both cases). **Missing:** the validator call (N-3.6). **R-N 2026-10-04:** `verdict::judge_metrics` is called by `promote_gated` check 5 and `GET .../verdict`; 9 `verdict::tests` pass. Gate 3 binding call (N-3.6) outside territory.

`BindingVerdict`: return or direction forecasts need walk-forward OOS R-squared > 0 with Clark-West p < 0.05; vol forecasts must beat HARQ on QLIKE; a `var_95` or `es_95`
consumer needs a passed ES backtest. A missing measurement is `NotComputable{reason}`, never a pass.

**Files:** `crates/model-registry/src/scoring_v2.rs`
**Acceptance:** each of the three rules has a passing and a failing case; an absent HARQ score yields `NotComputable`; the verdict type has no constructor from a bare boolean (mutation check: making the rule `>=` instead of `>` fails the boundary test).
**Traces:** MD-09, MD-10; FEAT-006 §7

### N-3.3 Purged MDA and clustered feature importance — M
`PARTIAL` 2026-10-04 — `importance::{purged_mda, correlation_clusters}`: permutation importance on caller-supplied purged test rows (it never chooses a test row), clustered by single-linkage correlation so a near-copy does not hide a signal, a shared permutation per cluster, standard error across folds, nothing claimed from a single fold. Tests: `the_informative_cluster_ranks_first_and_the_noise_cluster_contains_zero`, `clustering_credits_the_information_not_the_column`, `a_single_fold_has_no_error_so_nothing_is_claimed`. **Missing:** a caller (the report), and a real `SplitSpec::expand` fold feeding it. **R-N 2026-10-04:** trainer `app/importance.py` computes per-fold cluster increases (3 tests), `importance::summarize_fold_importances` summarises them, `GET .../importance` serves it. Not run end to end.

Replaces MDI in reports: permutation importance with purge and embargo, clustered by correlation so one signal in five columns is not five signals.

**Files:** `crates/model-registry/src/importance.rs` (new) or `apps/model-trainer`
**Acceptance:** on a planted dataset the informative cluster ranks first and the noise cluster's importance interval contains zero; the purge is the label horizon.
**Traces:** MD-09; FEAT-006 §7

### N-3.4 The vol-timed momentum baseline — S
`PARTIAL` 2026-10-04 — `scoring_v2::{vol_timed_momentum, complexity_vs_baseline}`; `the_baseline_never_reads_the_bar_it_trades`, `a_model_that_loses_to_the_baseline_is_flagged` (a tie is not a win). **Missing:** the scorecard does not call it (N-3.5). **R-N 2026-10-04:** `verdict::complexity` flags `complexity_unjustified` in the eval scorecard (test `a_return_model_that_loses_to_vol_timed_momentum_is_flagged`).

The simple baseline complex return models must beat (BACKTEST_SUITE v2 T5).

**Files:** `crates/model-registry/src/scoring_v2.rs`
**Acceptance:** a return model that loses to it is flagged `complexity_unjustified` in the scorecard.
**Traces:** MD-09; FEAT-006 §7

### N-3.5 A scorecard always carries its baselines — M
`NOT STARTED`

A scorecard that omits a baseline trained on the same folds is a type error, not a missing field.

**Files:** `crates/model-registry/src/scorecard.rs`
**Acceptance:** `compute_scorecard` refuses a metrics blob with no baseline rows; deflated scores use the model trial counter (N-2.10).
**Traces:** MD-09; FEAT-006 §7

### N-3.6 The validator enforces the usage rules at the Gate 3 binding — M
`PARTIAL` **R-N2 2026-10-05:** `crates/backtest/src/gates/model_binding.rs` (a trait seam, because `crates/model-registry` depends on `crates/backtest` and a direct call would cycle): `ModelBinding`, `ModelUsagePolicy`, `BindingOutcome::judge` (no constructor from a boolean; a binding naming no use is refused). `GateRunner::with_model_bindings` makes `enter(Significance | Vault)` return `GateError::ModelBindingRefused` while any binding is refused. Implementation: `crates/api/src/models_usage_policy.rs` (`RegistryUsagePolicy`, over `model_registry::manager::usage_verdict_pg`, the one reading of the evidence the promotion gate uses), and `gate_worker` refuses a `gate_advance` job whose manifest `model_bindings` fail, terminal `gate_failed` / `model_binding_refused`, before any evidence is gathered. Tests: backtest lib `gates::model_binding::tests` (5, mutation-checked: removing the check in `enter` fails the runner test), api lib `models_usage_policy::tests` (2, incl. no recorded evaluation is refused) and `gate_worker::tests::model_bindings_are_parsed_and_a_malformed_list_is_refused` (mutation-checked). **Still missing:** nothing populates `model_bindings`: the funnel and campaign driver do not know which models a candidate binds (that is Set U's strategy-to-model-slot resolution), so no production job carries the field yet; the "vol model loses to HARQ" and "no ES backtest" cases are the registry's `verdict` tests, not driven through the gate worker.

When a model or prediction series is bound in a strategy that enters Gate 3, the validator calls N-3.2 and refuses the binding on a failing or `NotComputable` verdict.

**Files:** `crates/strategy-engine` or the gate stack (outside this track)
**Acceptance:** a vol model losing to HARQ is blocked from sizing at Gate 3; a model with no ES backtest is blocked from ES sizing; a passing model proceeds.
**Traces:** MD-09, MD-10; FEAT-006 §7, M6
