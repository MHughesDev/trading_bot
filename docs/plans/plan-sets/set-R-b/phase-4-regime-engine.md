# Phase 4 — The regime engine and its learned tier

**Completion: 17% (1 / 6 tasks), plus 5 `PARTIAL`.**

**Requirement IDs:** FEAT-003 phase 3
**Spec:** [FEAT-003](../../../specs/FEAT-003-strategy-research-agent.md) §10, §15
**Depends on:** Set N N-2.4.
**Migration range:** 0214.

FEAT-003 §10: regimes as persistent states, labelled **filtered** (no look-ahead), feeding `RegimeConditional` studies, a `regime_strategy_prior` table and a regime gate move. Today `regime_jobs.rs`, `regime_search.rs`
and `regime_separation.rs` exist; whether they implement this contract is the first thing RB-4.1 settles.

---

### RB-4.1 Audit what the regime code does against FEAT-003 §10 — S
`DONE` 2026-10-04 — audit of the regime code against FEAT-003 section 10 (verdicts with file and line; evidence for RB-4.2 to RB-4.5):
- **Features** (Hurst, variance ratio, realised vol, ADX, autocorrelation, skew/kurtosis, volume z): **PARTIAL.** `crates/features/src/families/hurst.rs:31` (`hurst_rs`) and `crates/features/src/registry.rs:330,373,374` (`rolling_skew`, `autocorr`, `variance_ratio`) exist as feature families; the daily labeller reads **none** of them: `apps/platform/src/regime_jobs.rs:90-117` labels from a 21-day rolling volatility of the scope series only. No ADX, vol-of-vol or volume z-score family was found.
- **Model** (a persistent jump model, three regimes, penalty by information criterion, refit monthly): **MISSING before this set.** The platform ran only the rule tier, `rule_vol_tercile_v1` (`crates/api/src/knowledge.rs:323`, `backtest::gates::evaluators::vol_tercile_regimes`, `crates/backtest/src/gates/evaluators.rs:255-290`). **A defect:** that function takes its tercile cut points over the **whole window being labelled** (`evaluators.rs:269-275`), so the label at time t moves when later bars arrive: it is not a filtered label by the section's own standard, whatever its `knowledge_time`. `crates/api/src/knowledge.rs:331-338` says so ("a description of that period rather than a prediction within it"). Replaced for new labels by `research::regime_engine` (RB-4.2), which is filtered by construction.
- **Outputs** (`regime_labels` per bar; windows in the shape `VarySpec::Regimes` takes): **BUILT.** The table is `regime_causal.regime_state` (`migrations/0055_knowledge_plane.sql:81`, filtered probabilities, `model_version`, `knowledge_time`, `label` CHECKed to `low_vol|mid_vol|high_vol`), and `VarySpec::Regimes` is `crates/backtest/src/study/config.rs:66` with `RegimeConditional` at `:215`. The smoothed table is a separate schema with its own grants (`0055:96`, AT-42).
- **Prior** (`regime_strategy_prior`, a contextual-bandit table): **MISSING before this set** (built: migration 0214, `PriorStore`). The campaign context citing it is not wired.
- **Scope registry and validation**: **BUILT.** `regime_causal.regime_scope` (`migrations/0117_regime_scopes_and_separation.sql:29`, ADR-P3-07) and the out-of-sample separation test (`crates/api/src/regime_separation.rs:493`, `SeparationJob`), run daily after the labels by `regime_jobs.rs`.
- **Tools** (`get_regime_profile`, `get_regime_prior`): **MISSING before this set** (built: RB-4.3). `add_regime_gate`: the `Move` variant exists (`crates/research/src/propose.rs:87`) with its machine check named `Pending::RegimeLabels` (`:182`) and **no resolver**; built in RB-4.5.

Compare `regime_jobs`, `regime_search`, `regime_separation` and `regime_scopes` (migration 0117) with §10: features, model, outputs, prior. Record built, partial, missing.

**Files:** this file
**Acceptance:** each §10 bullet has a verdict with a file and line.
**Traces:** FEAT-003 §10

### RB-4.2 Statistical jump model fit and persisted `regime_labels` — L
`PARTIAL` 2026-10-05 (R-WIRE) — the daily job now calls the jump labeller: `apps/platform/src/regime_jobs.rs` runs `api::knowledge_regime_jump::JumpLabeller::run` after `write_rule_tier` for every scope (`jump_labels`; a failure is logged and does not stop the rule tier's separation test). Also fixed the RB-4.1 finding that the **rule tier was not filtered**: `backtest::gates::evaluators::vol_tercile_regimes` took its cut points over the whole window, so a day's label changed when later bars arrived; the cut points are now expanding (the terciles of the trailing volatilities of days up to and including `t`) and the trailing window is no longer clipped to the series length (ADR-RW-08). Test: `a_label_at_t_does_not_change_when_later_bars_arrive` (prefixes at six cut points equal the full run); mutation-checked (restoring whole-window cut points fails it). One Gate 11 fixture whose profitable window straddled a volatility-block boundary was moved inside one block, because a filtered label honestly splits such a window across two regimes. Earlier tests unchanged. **Still missing:** (2) the richer feature list (Hurst, variance ratio, ADX; the series here is daily returns and ADX needs highs and lows); (3) a run on real BTC daily bars and on a pgvector database (`regime_state.regime_vec` is `vector(16)`).

Three regimes, penalty chosen by the information criterion, refit monthly, labels filtered. Shares its implementation with Set N N-2.4.

**Files:** `apps/model-trainer` (outside this track)
**Acceptance:** regime length on daily BTC features has median at least 5 days; labels reproduce from stored features; a label at time t is unchanged when later bars arrive.
**Traces:** FEAT-003 §10, Phase 3

### RB-4.3 `get_regime_profile` and `get_regime_prior` tools — M
`PARTIAL` 2026-10-04 — `research::regime_prior::profile` (occupancy, row-normalised transitions, median stay, current label; an instrument with no labels is a typed refusal), `GET /api/knowledge/regimes/profile` and `/prior`, the `get_regime_profile` and `get_regime_prior` tools (the spec's names) and `tbot knowledge regime-profile|regime-prior`; the prior reports `n_obs` beside every number. Tests: `the_profile_reports_occupancy_transitions_and_the_current_label`, `an_instrument_with_no_labels_is_a_typed_refusal`, `a_study_updates_its_own_cell_and_the_prior_reports_its_observation_count`. **Missing:** the scope resolution and history read (`resolve_scope`, `RegimeStore::history`) against a database: they need `regime_causal.regime_state` (pgvector).

Occupancy, transitions and current label; top families per regime with `n_obs`.

**Files:** `crates/mcp-server/src/tools` (new file)
**Acceptance:** the profile of an instrument with no labels is a typed refusal; the prior reports its observation count.
**Traces:** FEAT-003 §11

### RB-4.4 `regime_strategy_prior` updates — M
`PARTIAL` 2026-10-05 (R-WIRE) — the regime-sliced study now updates the prior: `study_worker::fold_regime_prior` labels each COMPARE replicate window of the candidate with the filtered regime in force **at its start**, pairs it with that window's primary metric (success = the metric is positive) and calls `PriorStore::update` for `(tenant, universe, regime, strategy_type)`; `CompareWorker` calls it after the decision is logged, so a retry cannot fold the same windows twice, and a failure is advisory (ADR-RW-07). A window with no label, a failed run, a non-finite score, or an instrument with no configured scope folds nothing. Tests: `windows_are_folded_into_the_prior_by_the_regime_in_force_at_their_start` (scratch Postgres: low and high cells get n_obs 1 with the right n_success; a failed run and a NaN add nothing; a window before the first label is dropped; an unscoped instrument folds 0), `only_a_labelled_window_with_a_finite_score_reaches_the_prior`; mutation-checked (success always true fails the first). **Still missing:** the acceptance's last clause, that the first-generation hypothesis of a fixture campaign *cites* the prior: `get_regime_prior` exists as a tool, but whether a HYPOTHESIZE sub-run is shown or allowed it is `agent/subrun.rs`, which this track does not edit.

After each candidate's regime-sliced study, update the `(instrument, regime, strategy_family)` table. A contextual-bandit table, not a classifier. Migration 0214.

**Files:** `migrations/0214_regime_strategy_prior.sql`, `crates/research`
**Acceptance:** a regime-sliced study updates the right row; the table is `app_role`-granted; the first-generation hypothesis of a fixture campaign can cite the prior.
**Traces:** FEAT-003 §9, §10

### RB-4.5 The `add_regime_gate` move — M
`PARTIAL` 2026-10-04 — `research::regime_prior::resolve_regime_gate`, the `Pending::RegimeLabels` check of the `add_regime_gate` move: an unknown label is refused naming the known ones, a missing source is refused, and a known one returns the regime source it cites (`regime_causal.regime_state@<model_version>`); `POST /api/knowledge/regimes/gate` runs it against the scope's latest model version. **Missing:** applying the gate to a strategy definition (`crates/strategy-lang`, outside this track).

A structural move that adds a regime condition; validated like every move.

**Files:** `crates/strategy-lang` (outside this track), `crates/research`
**Acceptance:** the move produces a valid revision citing the regime source; an unknown regime label is refused.
**Traces:** FEAT-003 §8.3

### RB-4.6 The learned regime tier: HMM/HSMM with BOCPD — L
`PARTIAL` 2026-10-04 — `research::regime_learned`: a Gaussian HMM (Baum-Welch with scaled forward-backward; filtered posteriors only, the backward pass lives inside `fit` and returns parameters, ADR-RB-09), Bayesian online change-point detection (Adams-MacKay with a Normal-Inverse-Gamma Student-t predictive and a constant hazard; a break shows as posterior mass on short runs), and `LearnedGate`, the M7 trigger (ADR-RB-13): the rule tier answers, and says why, unless the separation test has passed; `GET /api/knowledge/tiers` reports `regime.answer.decision_tier`. Tests: ordered variances and sticky transitions, filtered labels unchanged when later data arrives, likelihood beats a single Gaussian, a variance break detected and quiet before it, no public function returns a smoothed posterior. **Missing:** the runtime A/B (logging both tiers' labels with `decision_tier` per decision) and a fit job; neither exists, and the trigger's inputs (a passed separation test on real data) have not been observed on this machine.

Behind a ledger-size trigger (ADR-P4-01): fit only when the regime-sliced study count exceeds the stated threshold. Online change-point detection (BOCPD) for regime breaks.

**Files:** `apps/model-trainer`
**Acceptance:** below the trigger the rule tier answers and says so; above it the learned tier is A/B'd against the rule tier by `decision_tier` logging.
**Traces:** ADR-P4-01; FEAT-003 §10
