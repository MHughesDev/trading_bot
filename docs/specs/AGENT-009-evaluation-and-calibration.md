# AGENT-009: Evaluation and Calibration — how we know a harness change made the agent better, per tier, before it ships

> **Status:** **Partially implemented** — Set S phase 4. Re-checked 2026-09-21.
>
> The scorecard and the tier measurement exist. The `judgement` suite of §3 — planted edges the agent must find, planted nulls it must reject — is the open half, and DONE-4 rests on it.
>
> **The live count is in [`docs/plans/plan-sets/set-S/phase-4-memory-and-evaluation.md`](../plans/plan-sets/set-S/phase-4-memory-and-evaluation.md), and the programme total is in [`docs/plans/PLAN.md`](../plans/PLAN.md).** It is deliberately not restated here: a number written into a spec is a number nobody recomputes, which is how this line came to say "ready to build" about code that was already serving traffic. `crates/invariants/tests/the_plan_counts_what_the_plans_say.rs` holds those two files honest; nothing holds a hand-copied number here.
**Date:** 2026-09-14
**Author:** Claude (Fable 5.1)
**Revises:** AGENT-004 (agent evaluation suite) — §2 architecture, §3 suites, §6 scorecard, §7 gate are replaced; §4 auditor suite stands
**Depends on:** AGENT-010 §5 (the effect log is the trajectory), AGENT-005 §7 (token calibration), AGENT-007 §4 (retrieval log), AGENT-008 (what a step is)
**Related:** ADR-0032 (`MultiStepEvidence`), `crates/llm/tests/local_multistep_eval.rs`, `crates/harness/src/trace.rs`, `docs/LOCAL_TIER_FINDINGS.md` §9 (the eval bug)

---

## 0. Why this spec exists

The one eval that exists (`crates/llm/tests/local_multistep_eval.rs`) needs a live Ollama and scored the local tier 0/10 because *the eval itself* drove exposure off the call count and made the recovery check unsatisfiable (`LOCAL_TIER_FINDINGS.md` §9). It nearly "confirmed" that a 7B could not multi-step. After the harness bugs were fixed it scored 10/10 twice. The lesson is that an evaluation of an agent harness is itself a harness, and it needs the same discipline: deterministic where possible, replayable, and measured against a reference rather than a belief.

Today there is no replayable trajectory (harness-owned tools are not traced, `PROGRESS.md` ADR-P0-18), no per-tier scorecard, no calibration of the token estimator (AGENT-005 §0), no measure of whether the agent *used* what it knew (memory recall), and no gate that stops a harness change from regressing a tier it was not tested on. `MultiStepEvidence` in the profile YAML is hand-written prose about a run someone did once.

This spec makes evaluation the thing that decides: what a tier is allowed to claim, which harness version ships, which playbook is admitted, which estimator constant is used.

## 1. Principles

- **P1 — Replay before rerun.** The loop is sans-IO (`harness::drive::Loop::next(Input) -> Vec<Effect>`), so a recorded trajectory replays exactly with a scripted model. Most evals run with no model at all; model-in-the-loop evals are the expensive minority.
- **P2 — Score the harness and the model separately.** A failure is attributed to a rung: routing, exposure, decode, validation, tool, memory, or the model's choice. "The agent failed" is not a finding.
- **P3 — Every profile claim has an eval that licenses it.** `tool_calling: multi_step`, `decode: select_fill`, `code_execution: full`, `effective_budget_tokens` — each is a measured value with the measurement recorded next to it, on named hardware.
- **P4 — Non-inferiority per tier, not on average.** A change ships when no tier's scorecard regresses beyond the tolerance and at least one improves. A frontier gain that costs the 7B is a regression.
- **P5 — Understanding is measured, not assumed.** Did the agent cite prior findings when they were relevant? Did it refuse when the evidence was absent? Did it ask a human at the right moment and not at the wrong ones? These are scores, with reference answers.
- **P6 — Cheap and continuous.** Replay evals run on every PR in seconds; model evals run nightly per tier with a cost cap; live shadow evals run on real sessions after the fact.

## 2. The trajectory

[DO] A trajectory is the ordered list of `(Input, Vec<Effect>)` pairs a `Loop` produced, plus the profile, the registry hash, the task, and the working state at each step (AGENT-008 §2). AGENT-010 §5 persists it as `agent_events`. Extend `crates/harness/src/trace.rs` `Event` (`:21`) with `CoreTool { name, args, reply }` and `StateSnapshot { step, working_state_hash }` so harness-owned tools and state are in the record (closes ADR-P0-18). `is_replayable` (`trace.rs:149`) must return true for every trajectory the platform records; a test asserts it on a sample of live runs nightly.

A **model tape** is the sequence of raw replies a model gave; replaying a trajectory means feeding the tape back through the same `Loop` version and asserting the same effects. A tape can be edited by hand to construct a case (a truncated reply, a wrong tool, a fake id).

## 3. Suites

Each suite is a directory `evals/<suite>/<case>/` with `task.json`, `profile` (name), `tape.jsonl` (for replay) or `model: live` (for model-in-the-loop), `expected.json`, and `score.rs` reference (a scorer name). Scorers live in `crates/harness/src/eval/`.

| Suite | Measures | Reference | Runs |
|---|---|---|---|
| `routing` | the tool the model named or searched for is exposed on the next step; no alphabetical truncation; `search_tools` resolves within one step | expected exposure sets per step | replay |
| `decode` | select→fill produces a valid call for each exposed tool on the tier; truncation re-asks; canary verdicts | 100% valid on the canary set; per-tool fill success ≥ 95% | live, nightly, per local tier |
| `budget` | AGENT-005 §2.2 arithmetic holds on every call (system + schemas + body ≤ num_ctx − reserve); compaction leaves handles; findings block is single | assertions | replay |
| `honesty` | planted oracle is killed by `run_oracle_test`, not rewarded; a fake id in `finish_task` is refused; a `suggestive` finding is never reported `established`; a seeded lookahead is caught by `leakage_audit`; the null comparison is never skipped before a gate | pass/fail per case | replay + live |
| `stopping` | lookup goals end in ≤ 3 calls and 0 trials; research goals reach `finish_task` with ids; barren loops wrap up; budget exhaustion buys the report turn | expected outcome + bounds | replay |
| `memory` | given a brief with a relevant established finding, the run cites it (recall); given a negative result with a met retry condition, the run mentions it; given a conflict, the run does not silently pick a side | `memory_retrievals.cited` and finish text | live |
| `hitl` | asks a human on `SealedHoldout`/`Promotion`; does not ask on reversible reads; classification precision/recall vs. a labelled action set | labelled set | replay |
| `critic` | the fresh-session critic's `blocking` flags on a labelled set of flawed/unflawed dossiers | precision ≥ 0.7, recall ≥ 0.8 | live, weekly |
| `research` (end-to-end) | the FEAT-003 chain: train → strategy → experiment → sweep → study → gate on a fixed dataset with a fixed seed; trials spent, N_eff, gate reached, cost | per-tier bounds | live, nightly for local_mid and frontier; weekly for local_small |
| `judgement` (AGENT-013 §10, added 2026-09-14) | **research decisions** against planted ground truth: a synthetic dataset with a known edge is reported `supported` and one with a known null `refuted`, never the reverse; a known-leaking feature is caught before `supported`; a planted dominant candidate is ranked first by `compare_candidates`; a dead idea already in memory is refused at intake; the workflow chosen matches the question's shape; the refinement chain stops at diminishing returns; the falsifier sealed is evaluable; yield is non-zero | pass/fail per case, per (tier, workflow); ≥ 30 cases per tier | replay where the tape is fixed; live nightly otherwise; gates a frontier profile's eligibility for continuous research and for `autonomous` (AGENT-014 §8) |
| `auditor` (AGENT-004 §4, stands) | truncation marking, no-wallclock, envelope coverage | assertions | CI |

## 4. Scorecard and gate

[DO] `evals/scorecard/<harness_version>/<profile>.json`: per suite pass rate, per rung failure attribution, cost (tokens, USD), median steps per task, cache hit rate (AGENT-005 §10.2), memory recall, HITL precision, critic precision, and — from AGENT-013 §10 — `judgement` pass rate, barren-run rate and cost per knowledge unit per workflow. Published by CI as an artifact and rendered in the Meridian UI's governance page.

**Gate (P4).** A PR touching `crates/harness`, `crates/api/src/agent`, `crates/mcp-server`,
`config/profiles`, the templates, or a playbook must show, for every profile with an existing
scorecard, that **no suite regressed** and that at least one improved (or the PR is labelled
`neutral` with a reason). Cost may not rise more than 15%.

"Regressed" is a statistical claim, and this platform does not accept those without an
interval. It would be indefensible for the spec that demands a deflated Sharpe ratio of a
strategy to gate itself on a bare two-point difference in a pass rate. So:

- **Deterministic (replay) suites are exact.** They have no sampling error: a case passes or
  it does not. Any newly failing case blocks the merge, full stop — no threshold, no
  interval, because there is nothing to estimate.
- **Stochastic (live) suites use an interval.** Regression means the upper bound of a
  one-sided 95% Wilson interval on the difference in pass rate is below zero, with a minimum
  of **n = 30 cases per suite per profile**. Below that the suite reports `underpowered` and
  can neither block nor clear a PR. A point estimate that moved without a significant
  interval is reported and does not gate.
- **The gate runs on every PR, so it is a repeated test.** Apply Benjamini-Hochberg across
  (profile × suite) within a PR, and record the number of gate evaluations per harness
  version so the platform can see its own multiple-testing exposure — the discipline the
  trial ledger applies to research, applied to the harness that judges it.
- **Cost is exact** (tokens and dollars are counted, not sampled) and needs no interval.

**A profile with no prior scorecard cannot be gated against itself.** Its first run
establishes a baseline, is labelled `baseline` rather than `pass`, and the profile reports
`unmeasured` until two scorecards exist. Otherwise introducing a new profile is the obvious
way around this gate.

Replay suites block the merge; live suites block the nightly promotion of the harness version
to `current`.

### 4.1 Flakiness is a score, not a nuisance

A pass rate hides variance, and variance is the thing that makes a gate lie. A live suite that
passes 95% of the time on Monday and 60% on Tuesday has the same mean as one that passes 78%
every day, and only the second is a system you can reason about — but under a pass-rate gate
the first will randomly block honest PRs and randomly clear regressions, and the response to
both is to stop believing the gate.

Every live suite runs **k = 3 repeats** per scorecard with different seeds, and the scorecard
carries `pass_rate`, `flakiness` (the spread across repeats) and `n`. A suite whose flakiness
exceeds 0.15 is reported `unstable` and is **excluded from gating** until it is fixed or its
cases are made deterministic — a gate that fires at random is worse than no gate, because it
spends the team's trust. Flakiness trends are published beside pass rates, since a suite
becoming flaky is usually the harness becoming non-deterministic, which is itself the finding.

### 4.2 The eval set has a holdout, for the same reason a strategy does

This platform's entire thesis is that tuning against the data you evaluate on produces a
number that means nothing. The eval suite is not exempt: every harness change is tuned until
the suites pass, the suites are small, and after twenty changes the harness is fitted to the
cases — and the scorecard says everything is fine, in exactly the way a curve-fitted backtest
says everything is fine.

- **20% of every suite's cases are sealed**, chosen once by hash, and are **not run on PRs**.
  Nobody tunes against a number they cannot see.
- The sealed set runs **on the nightly promotion of a harness version to `current`**, and its
  result is reported beside the open set's. A version whose open-set pass rate exceeds its
  sealed-set rate by more than the flakiness band is **fitted to its own evals** and is not
  promoted, whatever the open set says.
- The sealed set is re-drawn only when the suite grows by 50%, and the re-draw is recorded
  with the version that requested it — the same look-accounting the platform applies to a
  strategy's holdout (AGENT-006 §5.4), applied to itself.

If that sounds severe for an internal test suite: it is the identical argument the platform
makes to a researcher who wants one more look at a holdout, and a rule the platform will not
apply to itself is a rule it does not believe.

## 5. Calibration (the numbers the harness runs on)

| Value | Today | Calibrated by |
|---|---|---|
| bytes-per-token per model | 4.0 constant (`context.rs:123`) | AGENT-005 §7: provider `prompt_tokens` vs. harness estimate, median over 50 runs, stored in `agent_token_calibration`; the `budget` suite asserts the estimate is an over-estimate ≥ 95% of the time |
| `effective_budget_tokens` per local model | declared in YAML | a **context-pressure** eval: the `research` suite run at 25/50/75/100% of the declared budget; the value is set where quality (gate reached, honesty pass) is within 5% of the 25% point. Recorded in the profile as `effective_budget_evidence` |
| `tool_calling: multi_step` | `MultiStepEvidence` prose | generated by the `decode` + `routing` suites on named hardware; the YAML field becomes a pointer to the scorecard row. `effective_tool_calling` (`profile.rs:527`) keeps the device re-check |
| `max_exposed_per_step` | 24 / 8 / 6 | the `routing` suite at 4/6/8/12 exposure on each local tier: the smallest budget with no routing failures + 2 |
| `MIN_BODY_SHARE`, `max_plan_steps`, `MAX_BARREN_STEPS` | constants | swept in the `stopping` suite; changed only with a scorecard diff |
| finding `valid_until` policy | 90 days (AGENT-007) | the re-verification job's disagreement rate by age |

Every calibrated value carries `{value, measured_on: {model, device, harness_version, date}, suite_run_id}`; a profile with an uncalibrated claim is loaded with a warning and its tier is reported as `unmeasured` in the UI.

## 6. Live shadow evaluation

[DO] Nightly, replay the day's real trajectories (tapes) through the candidate harness version. Differences in effects are classified: same tools different order (neutral), extra tool (cost), missing tool (routing regression), different verdict (blocking). The candidate cannot become `current` with a blocking difference on a real session unless a human accepts it with a note. This is the non-inferiority check on the distribution that matters — the user's actual questions — at zero model cost.

## 7. Build order

| Phase | Items |
|---|---|
| 1 | `agent_events` as trajectory (AGENT-010 §5); `Event` extensions; model tape record/replay; `routing`, `budget`, `stopping` replay suites from the cases in LOCAL_TIER_FINDINGS §8–9 |
| 2 | `honesty` suite (oracle, fake id, seeded leakage); scorecard artifact; PR gate on replay suites |
| 3 | `decode` live suite per local tier; `MultiStepEvidence` generated; token calibration table |
| 4 | `memory`, `hitl`, `critic` suites once AGENT-007/008 phases 2–4 exist; context-pressure calibration |
| 5 | live shadow evaluation; nightly promotion gate |

## 8. Acceptance

1. Every live run's trajectory replays through the same harness version to identical effects.
2. The 0/10 eval bug from LOCAL_TIER_FINDINGS §9 exists as a replay case and passes.
3. A PR that regresses the 7B's routing pass rate by 3 points is blocked even if the frontier improves.
4. `config/profiles/*.yaml` contains no `MultiStepEvidence` prose; each local profile points at a scorecard row.
5. `agent_token_calibration` has a row per model with ≥ 10 runs; the `budget` suite reports the over-estimate rate.
6. The scorecard shows memory recall and HITL precision per tier.
7. Nightly shadow replay runs and its blocking differences are visible in the governance UI.
8. The `judgement` suite has ≥ 30 cases per tier; a harness reporting a planted null `supported` fails it; the scorecard shows `judgement`, barren-run rate and cost per knowledge unit per (tier, workflow); the sealed holdout (§4.2) applies to it. *(Added 2026-09-14, AGENT-013.)*

## 9. Non-goals

- Benchmarking foreign models for their own sake; only profiles the platform runs are scored.
- LLM-as-judge for correctness of research conclusions; the honesty suite uses planted ground truth, and the research suite uses fixed datasets with known answers.
- Replacing AGENT-004 §4's auditor suite.

## 10. Open questions

- Tolerance values (2 points, 15% cost) are starting points; revisit after ten scorecards.
- Whether the critic precision target should differ per tier (a checklist critic has different failure modes from a model critic).
