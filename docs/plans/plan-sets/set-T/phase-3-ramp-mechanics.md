# Phase 3 — Ramp mechanics

**Completion: 25% (1 / 4 tasks), plus 3 `PARTIAL` (T-3.1 to T-3.3: 2026-10-04 now scheduled hourly from the platform, Gate 16 renders the judged period, and the order gate honours a size between full and zero; still not run on real forward rows, which need the T-0.4 deployment).**

**Requirement IDs:** FE-RAMP-01…FE-RAMP-04, FE-G16.
**Spec:** ML platform SPEC §12.3 Gate 16 (capital ramp 10 / 25 / 50 / 100%); ADR-P2-27
(re-gate, step-down); migration 0053 (`capital_ramp`).
**Depends on:** Phase 2.
**Blocks:** nothing buildable — Stage 6 is a human decision (roadmap H-4).
**Migration range:** 0111–0112.

Gate 16 describes a ladder nothing climbs. This phase builds the mechanics so that **if and
when** roadmap gate H-4 (capital and liability) is answered, the path from a `strict_v1` pass to
capital is a mechanism someone reviews rather than a decision someone improvises.

**Nothing in this phase moves money.** Every task builds and tests the ladder against paper
deployments. Ring 3 has no tool, and this phase does not add one.

---

### T-3.1 The ladder, as a state machine — M
`PARTIAL` 2026-10-04 (track T-P) — **now exercised on rows, not on empty recorders; still `PARTIAL` because no real forward row exists yet.** Every earlier worker test used a deployment whose recorder had observed nothing, so the SQL that turns `mlops.forward_observation` and `mlops.divergence_halt` into period evidence (`ramp_worker::evidence`) had never run over a row. `deployment_path::a_period_is_judged_on_the_forward_rows_inside_it` (PG, as part of the 18 passing `deployment_path` tests) plants six deployments, each with a proposal behind it: a healthy one (29 observed bars inside, one at the period end counted, one at the start and one after the end not counted: `observed_bars == 30`) holds at full size with nothing applied and Gate 16's own verdict stored beside the judgement; a signal-match failure and a divergence halt inside the period each halve and name the reason in the stored judgement; 30 `backfill` rows count as zero observed bars (not computable, halves); 10 observed bars is not computable (halves); a halt raised after the period does not count against it. Mutation-checked: letting backfill rows into the evidence query fails the test (healthy no longer holds at the asserted count). `app_role` holds SELECT on both tables (`has_table_privilege`). Still missing: a live period over rows a real recorder wrote (needs the T-0.4 deployment, `BLOCKED:HUMAN`, runbook).

`PARTIAL` 2026-10-04 — **scheduled.** `health_jobs::spawn_ramp(pg, kill_switch)` (called once from `main.rs`, beside the gate it shares the kill switch with) runs `ramp_pass` hourly: `api::ramp_worker::run_pass` with the monitor's own effects. The two numbers the ladder needed (14-day period for a deployment that declared none, 20 observed bars to judge a period) are now a row (`ramp_policy`, migration 0190, **ADR-T3-04**) the pass reads through `RampPolicy::load`; `app_role` can read and cannot write it (REVOKE, because 0044's default privileges would let it; `the_ramp_policy_is_a_row_the_platform_reads_and_cannot_write` as `app_role`). Still: a live strict_v1 climb has never run (no forward rows until T-0.4).

Earlier: `PARTIAL` 2026-10-03 — `ledger::ramp::Ramp` (state machine: size in halvings, optional capital rung; `review` takes a `RampAuthority` to climb; `replay` folds recorded `ReviewEvent`s; skips, post-zero reviews and broken arithmetic are errors; `paper_v1` has no rung) with migration 0181 (`ramp_period`, `ramp_review`; a raise on `mlops.capital_authorisation` must cite exactly one passed `step_up` review). Tests: ledger `ramp::tests` (12) incl. `the_history_replays_from_events_to_the_same_state`, `a_paper_v1_deployment_is_never_on_the_capital_ladder`; PG `a_capital_raise_needs_a_passed_review_and_spends_it_once`, `no_configuration_skips_a_step_down`. **Missing:** a live strict_v1 climb has never run (needs forward rows from T-0.4), and the worker is not scheduled. ADR-T3-02: capital rung and size are two quantities (rejected: reuse rung step-down, since 25% to 10% is not 50%).

`capital_ramp` (migration 0053) driven as an explicit state machine: 10% → 25% → 50% → 100%,
one step per review period, each step requiring the review period's evidence to still satisfy
the profile that authorised the deployment. A step is an event, not a column update, so the
ladder's history is reconstructible.

Steps are permitted only for deployments whose profile can authorise capital — which
`paper_v1` cannot (T-1.2). On paper, the ladder runs against paper capital, which is how it
gets tested without money.

**Files:** `crates/domain/src/capital_ramp.rs`, `crates/ledger` events, migration 0111.
**Acceptance:** a step advances only with satisfying evidence from the completed review period;
the ladder's history replays from events to the same state; a `paper_v1` deployment cannot enter
the capital ladder at all; skipping a rung is impossible by construction.
**Traces:** SPEC §12.3 Gate 16; migration 0053.

### T-3.2 Gate 16 judges the step — M
`PARTIAL` 2026-10-04 (track T-P) — **now exercised on rows, not on empty recorders; still `PARTIAL` because no real forward row exists yet.** Every earlier worker test used a deployment whose recorder had observed nothing, so the SQL that turns `mlops.forward_observation` and `mlops.divergence_halt` into period evidence (`ramp_worker::evidence`) had never run over a row. `deployment_path::a_period_is_judged_on_the_forward_rows_inside_it` (PG, as part of the 18 passing `deployment_path` tests) plants six deployments, each with a proposal behind it: a healthy one (29 observed bars inside, one at the period end counted, one at the start and one after the end not counted: `observed_bars == 30`) holds at full size with nothing applied and Gate 16's own verdict stored beside the judgement; a signal-match failure and a divergence halt inside the period each halve and name the reason in the stored judgement; 30 `backfill` rows count as zero observed bars (not computable, halves); 10 observed bars is not computable (halves); a halt raised after the period does not count against it. Mutation-checked: letting backfill rows into the evidence query fails the test (healthy no longer holds at the asserted count). `app_role` holds SELECT on both tables (`has_table_privilege`). Still missing: a live period over rows a real recorder wrote (needs the T-0.4 deployment, `BLOCKED:HUMAN`, runbook).

`PARTIAL` 2026-10-04 — **Gate 16 takes the judgement.** `backtest::gates::evaluators::gate_16_capital_ramp(profile, current, verdicts, step: Option<&StepJudgement>)`: `None` (the research-time stack, which passes `None` in `stack.rs`) states eligibility to request the next rung on the sixteen verdicts and says no period has judged a step; `Some(NotComputable)` is **inconclusive and never a pass**, carrying the judgement's reason; `Some(Fail)` stays; `Some(Pass)` rises only if the judgement permits a step up and otherwise reports the hold. Eligibility is checked first and no period overrides it (`a_passed_period_cannot_override_eligibility`: a flawless `paper_v1` period still moves nothing). `api::ramp_worker::review_one` calls it for every judged period and stores the Gate 16 outcome beside the judgement it was rendered from (`evidence.gate_16`), so the evaluator has a caller. Tests: `gates::evaluators::tests::gate_16_renders_the_judged_review_period`, `a_passed_period_cannot_override_eligibility`, and the three existing Gate 16 tests with the new argument. Unproven on real forward rows.

Earlier: `PARTIAL` 2026-10-03 — `ledger::ramp_gate::judge` gives Gate 16 an evidence source: the period's observed forward bars, divergence halts, signal match, slippage, turnover, rejects and profile pass; `Pass` / `Fail` / `NotComputable{reason}` (an incomplete or unobserved period is never a pass); evidence and judgement stored whole with a sha256 in `ramp_review` and re-judged to the same verdict (`the_judgement_reproduces_from_the_stored_evidence`, `a_failing_deployment_is_halved_each_period_and_reaches_zero`); a step-up with a failing verdict is refused in `Ramp::review` (`a_failing_gate_16_never_reaches_the_state_machine_as_a_step`). **Missing:** `backtest::gates::evaluators::gate_16_capital_ramp` (outside territory) does not yet call `judge`; hook: have it take a `StepJudgement`. Unproven on real forward rows.

Gate 16 currently has no evidence source. Give it one: the completed review period's forward
observations (Phase 0), the divergence record (Phase 2), and the profile's thresholds. A step-up
is a gate verdict like any other, with its evidence stored, and it renders `NotComputable` with
a reason rather than passing when the period is incomplete.

**Files:** `crates/backtest/src/gates/`, migration 0112.
**Acceptance:** Gate 16 renders a verdict from a completed review period and `NotComputable`
with a stated reason from an incomplete one; the verdict's evidence is stored and reproducible;
a step-up with a failing Gate 16 is refused at the state machine, not just warned about.
**Traces:** SPEC §12.3 Gate 16; HANDOFF §5.3 (Gate 16's missing input).

### T-3.3 Step-down: 50% per period, to zero — M
`PARTIAL` 2026-10-04 (track T-P) — **now exercised on rows, not on empty recorders; still `PARTIAL` because no real forward row exists yet.** Every earlier worker test used a deployment whose recorder had observed nothing, so the SQL that turns `mlops.forward_observation` and `mlops.divergence_halt` into period evidence (`ramp_worker::evidence`) had never run over a row. `deployment_path::a_period_is_judged_on_the_forward_rows_inside_it` (PG, as part of the 18 passing `deployment_path` tests) plants six deployments, each with a proposal behind it: a healthy one (29 observed bars inside, one at the period end counted, one at the start and one after the end not counted: `observed_bars == 30`) holds at full size with nothing applied and Gate 16's own verdict stored beside the judgement; a signal-match failure and a divergence halt inside the period each halve and name the reason in the stored judgement; 30 `backfill` rows count as zero observed bars (not computable, halves); 10 observed bars is not computable (halves); a halt raised after the period does not count against it. Mutation-checked: letting backfill rows into the evidence query fails the test (healthy no longer holds at the asserted count). `app_role` holds SELECT on both tables (`has_table_privilege`). Still missing: a live period over rows a real recorder wrote (needs the T-0.4 deployment, `BLOCKED:HUMAN`, runbook).

`PARTIAL` 2026-10-04 — **scheduled, and the size now reaches the order gate.** (1) `risk::RiskGate::check` reads `KillSwitch::scale_pct` for the order's strategy and cuts an order that would **add exposure** to that percentage, rounded **down** to a whole lot (**ADR-T3-03**): an order that scales to less than one lot is refused (as `StrategyHalted` with the reason, because `RiskRejection` is `domain`'s) and never rounded up; an order that **reduces** the position (`reduce_only`, or the opposite side and no larger than the position) is never cut, so a strategy being sized down can still get out; and the scaling runs before the idempotency cache, so a redelivery after a step-down is cut at the size current when it arrives. Rejected: scaling the position cap instead (a strategy with a small position and a large order would be unaffected), and refusing every order above the scaled cap (turns a step-down into a halt). Tests in `risk::gate::tests`: `a_strategy_scaled_to_half_trades_half_size`, `a_scaled_size_rounds_down_to_a_whole_lot_and_never_up`, `an_order_that_scales_to_less_than_a_lot_is_refused_not_rounded_up`, `an_order_that_reduces_the_position_is_never_cut`, `a_redelivered_intent_is_scaled_at_the_size_current_when_it_arrives`. (2) **The unmeasured policy (ADR-T3-04).** A deployment with **no forward recorder** cannot produce evidence whatever it does, so the pass **flags it every hour and does not step it down**: no period opens, no review is written, no size is applied (`an_unmeasured_deployment_is_flagged_and_not_halved`; `PassReport.unmeasured`, logged at warn with the ids). Rejected: halving every unmeasured deployment as ADR-P2-27 can be read, which punishes a missing measurement and walks every hand-run deployment to zero in 56 days before anything has been measured. A deployment **with** a recorder and fewer than `min_observed_bars` observed in a period is `NotComputable` and still halves (`a_failing_deployment_is_halved_each_period_and_reaches_zero`, now on deployments that have a recorder): a recorder that produced nothing for a whole period is itself a failure. Remaining: the 14-day / 20-bar numbers are floors chosen in ADR-T3-04, not yet exercised on real forward rows.

Earlier: `PARTIAL` 2026-10-03 — `ramp_worker::run_pass` opens first periods, reviews every ended period, chains the next from the recorded end, and applies each size idempotently every pass through the monitor's own controls (scale below full, halt and withdraw at zero). A failing or unmeasurable period halves: 1, 1/2, 1/4, 1/8, zero after four (`a_failing_deployment_is_halved_each_period_and_reaches_zero`, ledger `a_failing_deployment_halves_each_period_and_is_zero_after_four`); the DB refuses holding through a failure and a skipped or drifting period (`no_configuration_skips_a_step_down`). ADR-T3-01: four failing periods to zero, `scale_pct` rounds down (rejected: infinite halving). **Missing:** scheduling (hook for `health_jobs::run_once`) and the order gate honouring `KillSwitch::scale_pct` for sizes between full and zero (today only halt/zero reaches the gate; that is the MONITOR track's mechanism). Note: with no forward recorder (T-0.4) every deployment is unmeasured and would halve, which is ADR-P2-27's intent; decide that is wanted before scheduling.

ADR-P2-27's other half, and the one that actually applies today. An ungated or failing
deployment may not increase size, holds for one review period while a `strict_v1` or `paper_v1`
pass is attempted with its record, and **steps down 50% per period to zero** if it fails.

The step-down is automatic. It is not a recommendation on a dashboard, because the whole point
of ADR-P2-27 was that nothing gets to sit at its current size indefinitely on the strength of
never having been measured.

**Files:** `crates/domain/src/capital_ramp.rs`, a scheduled review worker.
**Acceptance:** a deployment that fails its review period is halved automatically at the period
boundary and reaches zero after the expected number of periods; the schedule is driven by
recorded period boundaries, not wall-clock drift; a step-down is recorded as an event with its
reason; no configuration permits skipping a step-down.
**Traces:** ADR-P2-27; D-T4.

### T-3.4 The capital gate is a document, not a code path — M
`DONE` 2026-10-03 — `docs/governance/capital-gate.md` (location per the track brief; the phase file said `docs/operations/`) names H-4 (broker, money, liability), a strict_v1 pass, the divergence record, the drill log, the 10% start, ceilings and the named decider (left unnamed). `crates/api/tests/no_capital_path.rs` (4 pass): exhaustive `match` on `VenueClass`, every venue CHECK paper, every deployment entry point on a reviewed list with the recording SQL hardcoding paper, no venue field on `NewDeployment`.

The final task of this set is deliberately not code. Write `docs/operations/capital-gate.md`:
the checklist that must be true before any capital deployment is even proposed — roadmap H-4
answered in writing (whose broker, whose money, who is liable), a `strict_v1` pass with its full
verdict, the fleet's divergence record, the drill log, the ramp starting at 10%, and the named
human who decides.

Then assert in code that no path exists to a capital deployment today: a test enumerating every
deployment entry point and asserting each produces a paper venue class.

**Files:** `docs/operations/capital-gate.md`, `crates/api/tests/no_capital_path.rs`.
**Acceptance:** the document exists and names every precondition including the human decision;
the test enumerates every entry point and asserts no capital venue is reachable; adding a capital
venue class fails that test, so enabling capital is a deliberate, reviewed change rather than a
configuration discovery.
**Traces:** ROADMAP §4 Stage 6, §8 H-4; Q-4; ADR-0025; D-12.

---

## Phase exit

- The ladder and the step-down are mechanisms with event histories, tested against paper capital.
- Gate 16 has an evidence source and refuses incomplete periods.
- The route to capital is written down, and code asserts that it is currently closed.
