# Phase 5 — Learned model tiers, FEAT-003 phases 2 to 4 and the visual pipeline builder

**Completion: 20% (2 / 10 tasks), plus 2 `PARTIAL`, 6 `BLOCKED:UPSTREAM`.**

**Requirement IDs:** ADR-P4-01, FEAT-003 phases 2 to 4
**Spec:** [FEAT-003](../../../specs/FEAT-003-strategy-research-agent.md) §15; ML pack PHASE-2-5-PLAN §4
**Depends on:** Phases 3 and 4.
**Migration range:** none.

Each learned tier of the research-process models is gated on a ledger-size trigger (ADR-P4-01); building one on a ledger too small to train it is how you get a confident wrong model. The triggers are written here
so that 'not yet' is a computed fact. FEAT-003's outer loop has partly landed under another name (`mlops.campaign`, the campaign workers); this phase lists what its spec still owes.

---

### RB-5.1 M3 learning-curve extrapolator — L
`BLOCKED:UPSTREAM` 2026-10-04 — the trigger is now **computed**, not asserted: `research::tiers::decide(M3, counts)` over `GET /api/knowledge/tiers` (at least 1,000 partial curves with at least 20% run to completion; the curve count is read from `mlops.training_metric`, the completion share has no table and is reported "not measured", which is not met). Below it the median stopping rule answers, and the response names the shortfall. The learned extrapolator itself waits for the data.

Trigger: at least 1,000 partial curves with at least 20% running to completion. The median stopping rule answers until then.

**Files:** `crates/jobs`, `crates/ledger`
**Acceptance:** below the trigger the stopper is the rule tier; above it, the learned tier runs shadowed first.
**Traces:** ADR-P2-10, ADR-P4-01

### RB-5.2 M4 config-to-outcome surrogate — L
`BLOCKED:UPSTREAM` 2026-10-04 — trigger computed from the ledger (at least 1,500 runs over at least 30 tasks: `count(*)` and `count(distinct experiment_id)` over `mlops.trial`) by `tiers::decide(M4, ...)`; the searcher keeps its own model below it (`two... one_short_of_any_clause_keeps_the_rule_tier`). The surrogate itself waits for the data.

Trigger: at least 1,500 runs over at least 30 tasks.

**Files:** `crates/research`
**Acceptance:** the trigger is computed from the ledger, not asserted; the searcher uses the surrogate only above it.
**Traces:** ADR-P4-01

### RB-5.3 M5 gate pre-screener — L
`BLOCKED:UPSTREAM` 2026-10-04 — trigger computed (at least 500 gated candidates and at least 100 passes, from `mlops.gate_verdict`); the safety property is built as a pure check, `tiers::screener_recall` (a pre-screener may not reject what a gate would pass, to 99% recall on a held-out set; mutation: one extra rejected pass drops recall to 0.98 and the check fails). The pre-screener itself waits for the data.

Trigger: at least 500 gated candidates and at least 100 passes; monotone at 99% recall.

**Files:** `crates/backtest/src/gates`
**Acceptance:** the pre-screener never rejects a candidate a gate would pass above the stated recall on a held-out set.
**Traces:** ADR-P4-01

### RB-5.4 M8 family recommender — L
`BLOCKED:UPSTREAM` 2026-10-04 — trigger computed (at least 100 tasks); `Model::M8.per_tenant_only()` is the one per-tenant-always entry (asserted unique), and `ledger_counts` reads under `ledger::pg::tenant_tx`, so no cross-tenant read exists in its data path. The recommender itself waits for the data.

Trigger: at least 100 tasks; per-tenant always (it is the one that would broadcast edge).

**Files:** `crates/research`
**Acceptance:** no cross-tenant read exists in its data path (a test).
**Traces:** ADR-P4-01

### RB-5.5 M9 proposal ranker — L
`BLOCKED:UPSTREAM` 2026-10-04 — trigger defined (at least 3,000 ranked items in at least 100 groups); there is **no ranking log table**, so both counts are reported "not measured" and the rule tier (surrogate mean plus k sigma) stays. The ranker waits for the data and for a ranking log.

Trigger: at least 3,000 ranked items in at least 100 groups.

**Files:** `crates/research`
**Acceptance:** the ranker's acquisition rule is logged beside the rule tier's choice.
**Traces:** ADR-P4-01

### RB-5.6 M13 step critic — L
`BLOCKED:UPSTREAM` 2026-10-04 — trigger computed (at least 3,000 labelled steps, from `mlops.trajectory_label`); judges triage and gates decide, unchanged. The critic waits for the data.

Trigger: at least 3,000 labelled steps. Judges triage; gates decide.

**Files:** `crates/harness`
**Acceptance:** the critic never changes a gate outcome.
**Traces:** ADR-P4-01; ADR-015

### RB-5.7 FEAT-003 Phase 2: islands, feature map and bandit allocation — L
`PARTIAL` 2026-10-04 — `research::campaign`: a feature-map `Archive` (best candidate per behavioural cell, duplicates by `config_hash` never compete), `Islands` (independent archives, ring migration every period, migrants deduplicated), UCB1 `next_arm` allocation with seeded tie-breaks, and `campaign_pbo` over `backtest::stats::probability_of_backtest_overfitting`. The fixture acceptance as a test: `a_two_hundred_backtest_fixture_campaign_fills_cells_and_is_not_overfit` (200 candidates occupy at least five cells, `campaign_pbo` at most 0.5). **Missing:** the campaign driver using them (`campaign_driver.rs`, outside this track), and the insight curation.

`crates/research/campaign`: islands, the feature-map archive, de-duplication, bandit allocation, ledger and insight curation. **Check first** what `campaign_driver.rs` already does.

**Files:** `crates/research`
**Acceptance:** a 200-backtest fixture campaign occupies at least 5 distinct feature cells with `campaign_pbo` at most 0.5.
**Traces:** FEAT-003 §15 Phase 2

### RB-5.8 FEAT-003 Phase 4: multi-instrument campaigns, refit scheduling and calibration-drift alerts — L
`PARTIAL` 2026-10-05 (R-WIRE) — the calibration-drift alert now reaches the inbox: `routes/alerts_inbox.rs` has a `calibration` source (`calibration_drifts` reads each model's last 12 `model_quality_series` windows and runs `research::refit::coverage_drift`, so the rule is not restated; one warning item per drift, with an id derived from the model and the window that completed the pair so acknowledge and dismiss refer to the same item, and a dismiss of a derived id is checked against what is drifting now), and migration 0245 widens `alert_inbox_state.source` to `('monitor','price','calibration')` and restates that `app_role` has no DELETE (ADR-RW-09). Earlier: `split_budget`, `refit_due`, `coverage_drift`, `cost_ladder_verdict`, cancellation. Tests: 3 unit (`two_consecutive_bad_windows_raise_one_calibration_item_with_a_stable_id`, `thin_windows_neither_raise_nor_hide_a_drift`, the source/verify check), `a_forecaster_whose_coverage_left_tolerance_twice_is_a_calibration_item` (scratch Postgres) and `calibration_is_an_admitted_source_and_delete_is_still_revoked` (under `SET LOCAL ROLE app_role`: calibration stored, a fourth source refused, DELETE denied; mutation-checked by narrowing the CHECK). **Still missing:** (1) the driver running multi-instrument campaigns with `split_budget` (a campaign names one `universe_ref` today); (2) two one-line frontend edits outside this track so the inbox page labels the new source (in the report).

Plus cost-model ladders inside sweeps (`CostSweep` as a gate input).

**Files:** `crates/research`, `crates/api`
**Acceptance:** cancelling a campaign stops its sweeps within one poll interval; a calibration drift raises an alert in the inbox.
**Traces:** FEAT-003 §15 Phase 4

### RB-5.9 FEAT-003 Phase 1 deferrals: fidelity racing and `sweep_max_parallel` — M
`DONE` 2026-10-05 (R-WIRE) — `ResearchManager::new` is now given `research::refit::sweep_slots(backtest::manager::MAX_CONCURRENT_RUNS, research::refit::sweep_max_parallel())` instead of the literal 2 (`crates/api/src/state.rs`; `MAX_CONCURRENT_RUNS` made `pub`): at most `SWEEP_MAX_PARALLEL` (default 4) sweeps and never the last backtest slot, so a campaign cannot hold every slot an interactive backtest needs. Test: `sweeps_are_given_the_slots_that_leave_one_free_for_interactive_backtests` (the call site is read from `state.rs` and the computed slots are fewer than the backtest permits). Earlier: `racing_report` (racing off below rho 0.8 and the report says so), `sweep_slots`'s floor and cap tests.

Racing between fidelity tiers enabled only where Spearman rho of adjacent tiers is at least 0.8; `sweep_max_parallel` default 4.

**Files:** `crates/research/src/calibration.rs` (additive)
**Acceptance:** racing is off where rho is below 0.8 and the report says so; one campaign cannot starve interactive backtests.
**Traces:** FEAT-003 §7.5, §7.6

### RB-5.10 The visual pipeline builder — L
`DONE` 2026-10-04 — **closed as "not yet", count 0.** ADR-P5-03 defers the visual pipeline builder until a third pipeline-stage type exists. A scan of `crates/`, `apps/`, `frontend/src` and `migrations/` for pipeline-stage type identifiers finds none (the 2026-09 Meridian rebuild removed the old Pipelines page; `/mlops` composes models, not stages). `crates/invariants/tests/the_pipeline_builder_waits_for_a_third_stage_type.rs` keeps the closure honest: it fails, naming the files, the day three such identifiers exist, which is the signal to re-open this task.

Deferred by ADR-P5-03 until a third pipeline-stage type exists; this task is the revisit. **Check first** whether a third stage type has appeared.

**Files:** `frontend/src` (outside this track)
**Acceptance:** the builder exists only if the trigger holds; otherwise the task is closed as 'not yet' with the count.
**Traces:** ADR-P5-03
