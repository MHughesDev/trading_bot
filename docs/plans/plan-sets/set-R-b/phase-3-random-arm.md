# Phase 3 — The shadow random-search arm (EV-09)

**Completion: 67% (2 / 3 tasks), plus 1 `PARTIAL`.**

**Requirement IDs:** EV-09
**Spec:** [BS-007 11](../../../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/11_EVALUATION_STANDARD.MD) §8, §9
**Depends on:** Set Q Q-1.6 consumes it.
**Migration range:** none.

A campaign's results mean something only against what random search with the same budget finds. EV-09 makes that arm mandatory: every campaign runs a matched-budget random arm beside the agent's, on the same sealed metric,
and reports both.

---

### RB-3.1 A matched-budget shadow random arm per campaign — L
`PARTIAL` 2026-10-05 (R-WIRE) — now reached by production code. `CampaignWorker::ensure_arm` creates the arm with the campaign, before any EXPERIMENT (budget = the declared sweep size, `max_runs`; seed from the campaign id; idempotent), and the first EXPERIMENT (`study_worker::record_arms`) records both arms: the agent's per-sample scores from its sweep and a second sweep of the **declared** space (no narrowing: ADR-RB-08), random sampler, the arm's seed, the same fixed parameters and the same budget, through the same evaluator, so its Runs are counted trials under the campaign's experiment (INV-1). Per-sample scores travel in-process (`SweepReport::sample_scores`, `#[serde(skip)]`) and are written under `knowledge.write` (ADR-RB-11, ADR-RW-05); both arms are padded to the budget with the failure value (ADR-RW-04). Idempotent: a retried job completes a half-written arm and never doubles it (`ArmStore::record_scores` is `ON CONFLICT DO NOTHING`). Tests: `the_driver_creates_the_arm_and_the_first_experiment_records_both_matched_arms` (8 and 8 scores, the random sweep has no narrowing, the `Random` sampler and the arm's seed, a later generation records nothing, the job result carries no `sample_scores`), `a_retried_experiment_completes_the_arm_and_never_doubles_it`, `sample_scores_are_kept_in_process_and_never_serialised`; mutation-checked (giving the arm the agent's narrowing fails the first). **Still missing:** (1) a run through the real `SuiteEvaluator` against the Backtest Suite and ClickHouse (every test here uses a fake engine, so "the arm's trials appear in the ledger" holds by construction and is not observed); (2) the arms are matched on the **first** generation only (ADR-RW-03), so a campaign that runs many generations compares its first sweep to random search, not its whole spend.

The arm samples from the same declared search space with the same trial budget and the same evaluation, through the platform sampler. Its trials are counted (INV-1) under the campaign.

**Files:** `crates/research` (additive), `crates/api/src/campaign_driver.rs` (outside this track)
**Acceptance:** a campaign run produces two arms with equal trial counts; the arm's samples contain no agent-chosen value; the arm's trials appear in the ledger.
**Traces:** EV-09; BS-007 11 §8

### RB-3.2 Report the arms together (`compare_to_random`) — M
`DONE` 2026-10-04 — `GET /api/knowledge/campaigns/{id}/compare-to-random`, the `compare_to_random` tool and `tbot knowledge compare-to-random`: both arms' sealed-metric quantiles, the bootstrapped best-of-budget distribution of random search (a smoothed bootstrap, so the percentile is not pinned to 1.0 under the null: `an_agent_that_only_matches_random_ranks_near_the_middle...` averages over 40 null campaigns) and the agent's percentile in it; a campaign with no arm, an incomplete arm or unequal budgets refuses with 409 `no_matched_arm` (`a_campaign_with_no_arm_or_an_incomplete_one_cannot_report_or_reach_a_verdict`, and the pg flow).

Feeds Set Q Q-1.6: the agent's rank in the random arm's distribution on the sealed metric.

**Files:** `crates/research`, `crates/mcp-server`
**Acceptance:** the report prints both arms' sealed distributions and the percentile; a campaign with no arm refuses to report.
**Traces:** EV-09

### RB-3.3 A campaign cannot reach a verdict without its arm — S
`DONE` 2026-10-05 (R-WIRE) — `backtest::gates::random_arm_gate` (`ArmRequirement`, `apply_to_stack`: Gate 3 and every later gate are refused, naming the arm and what the gate itself read; gates 1 and 2 stand; an unreadable arm is a refusal, never a pass) is applied by `gate_worker` immediately after `evaluate_stack` for a candidate with a `campaign_id`, from `knowledge_routes::arm_gate`, so the one rule stays `research::random_arm::gate_check` and `backtest` depends on neither `research` nor `api` (ADR-RW-06). Tests: `a_completed_arm_changes_nothing_and_no_requirement_is_not_a_requirement`, `a_missing_or_incomplete_arm_refuses_gate_3_and_every_later_gate_naming_the_arm`, `an_unreadable_arm_is_a_refusal_not_a_pass` (`cargo test -p backtest --lib gates::` 134 passed); the predicate against a real arm in `the_driver_creates_the_arm_and_the_first_experiment_records_both_matched_arms` (refused "0 of 8" before the arm runs, accepted after). Mutation-checked: moving the cut from gate 3 to gate 4 fails two tests. Not covered: the older `GateRunner` funnel (`suite.rs`), which has no campaign id; the recorded verdict is the stack's.

Gate 3 for a campaign candidate requires the arm to have completed.

**Files:** `crates/backtest/src/gates` (outside this track)
**Acceptance:** a campaign whose arm is incomplete cannot pass G3 and the reason names the arm.
**Traces:** EV-09, EV-07
