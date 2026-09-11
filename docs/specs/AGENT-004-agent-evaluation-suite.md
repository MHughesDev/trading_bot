# AGENT-004: Agent Evaluation Suite

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0024, ADR-0025, ADR-0028
**Derived from:** BS-007 [16_AGENT_EVALS](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/16_AGENT_EVALS.MD),
[02 §7 scorecard](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/02_AGENT.MD),
[04 §10](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/04_CONTEXT_AND_COST.MD)
**Plan set:** L (skeleton: noise, planted-edge, auditor suite, robustness), then extended
by every set
**Crates and apps:**
- new `evals/` (task definitions, graders, runner, in Python);
- `crates/jobs` (`eval_task` kind);
- DATA-005 synthetic venue;
- CI workflows.

---

## 1. Purpose

Make "the best possible agent" measurable, and make it the release gate for:
- every change to the harness, prompts, tools, skills, model or efficiency rules;
- skill admission and retirement (AGENT-003).

## 2. Architecture

```
evals/
  tasks/<suite>/<task_id>.yaml   # prompt, project setup (synthetic instruments, cutoff), budget, grader
  graders/                        # code graders (python) + model-grader rubrics
  runner/                         # creates an isolated eval project per trial, starts an agent session
                                  #   via the orchestrator (AGENT-001), collects report/events/usage
  suites.yaml                     # suite → task list, seeds, cadence, held-out flags
  results/  (artifact store)      # per-run scorecards
```

- **Each trial:**
  1. create a fresh eval project, with synthetic instruments generated from the task seed
     (DATA-005 §9);
  2. run a session with the production model, effort and image;
  3. grade the report, events, registries and usage;
  4. archive the transcript.
- Trials run as `eval_task` jobs under a platform eval budget, never a user's.
- **Hidden information:** generator parameters and planted-edge specifics are never
  visible to agent tokens. Catalog metadata for eval instruments is redacted.

## 3. Suites

| Suite | Construction | Grader | Metric |
|---|---|---|---|
| `noise` | `garch_t`, `merton_jump`, `regime_switch` with no signal | Code: `final_report.outcome` ≠ vaulted, and no candidate passes G3 | FDR, reported pass^k |
| `planted_edge` | `planted_ar1` (φ ∈ {0.02, 0.05, 0.1}), `planted_hour_drift`, `planted_vol_breakout`, `planted_carry` | Code: a candidate implementing the planted mechanism passes G3 | Power curve (pass@k) |
| `leakage_traps` | Tasks seeded with a look-ahead feature, overlapping labels, a model trained into the test window, revised-data use, costs off, collider filter, survivorship universe, post-hoc hypothesis, complexity-as-vol-timing | Code: flagged by the agent (report caveat or rejection) or by Gate 0 / validators | Catch rate |
| `qa` | Questions with computed ground truth (RV, drawdowns, event studies vs matched random), on synthetic and fixed real windows | Code: numeric tolerance; model grader for caveats | Accuracy |
| `modelling` | Synthetic series where a known feature predicts the target at a known OOS R² | Code: agent's OOS R², CW result and baseline comparison vs truth | Accuracy, honesty |
| `robustness` | Injected tool failures, timeouts, worker loss, a platform restart mid-session | Code: completes or resumes; no duplicate manifest hashes; trial counts consistent | Pass rate |
| `efficiency` | *amnesia* (ids and verdicts planted early; checked after 2 forced compactions), *duplicate_trials*, *refetch* | Code | Pass / rate |
| `report_quality` | All suites | Model grader (Batch API), monthly human transcript review | Rubric |

## 4. Auditor suite (no agent; CI)

`evals/auditor/` holds pairs of (violating, clean) fixtures, run directly against
Gate 0, the truncation test, the prediction-series overlap check, the `data_qc` gate and
the `final_report` validator:
- look-ahead features;
- `bfill` and centred windows;
- unpurged overlapping labels;
- a prediction series with a training-window overlap;
- revised-data use;
- costs disabled;
- grade-D data;
- uncited and mismatched report numbers;
- a skill with a tuned constant.

**Target:** 100% of violations caught, 0 false rejections on the clean twins. Runs on
every PR touching `crates/backtest`, `crates/research`, `crates/features`,
`crates/api` (reports and data) or `crates/agent-orchestrator`.

## 5. Protocol

- **Seeds:** ≥ 3 per task per release run, with fresh synthetic seeds every run.
- **Held-out tasks:** 30–40% of each suite is flagged held-out. It is never used to tune
  prompts, CLAUDE.md, tool descriptions, skills or skill descriptions. The skill-trigger
  eval sets are separate.
- **Settings:** always the production model, effort and image.
- **Metrics:** pass@k for discovery tasks; pass^k for noise and honesty rules.
- **Transcript triage:** every failure is triaged for grader bugs before being counted
  (the triage label is stored).

## 6. Scorecard (published per harness version)

| Metric | Target |
|---|---|
| FDR on `noise` (pass^k) | ≤ gate nominal + 2 pp |
| Uncited-number rate | 0 |
| Power at each planted strength | Tracked; no regression beyond −2 pp (95% CI) |
| Q&A accuracy | Per-task tolerance; non-inferior |
| Leakage-trap catch rate | 100% |
| Agent vs random-search arm (campaign tasks) | Reported; must beat it to claim search value |
| Robustness pass rate | 100% resume with no duplicate trials |
| Dollars per correct verdict; cache hit rate; tokens and tool calls per task | Tracked |
| Skill trigger precision/recall; leave-one-out contribution | Tracked (AGENT-003) |

## 7. Non-inferiority gate

Any change labelled `efficiency`, `cost` or `harness` in its PR must attach a suite run
meeting all of:
- FDR ≤ nominal + 2 pp;
- power ≥ baseline − 2 pp (lower 95% CI bound);
- Q&A non-inferior (margin 2 pp);
- trap catch rate unchanged;
- uncited rate 0;
- `amnesia` and `duplicate_trials` pass.

The margins are pre-registered in `evals/gates.yaml`.

## 8. Cadence and cost control

| Cadence | Scope |
|---|---|
| Per relevant PR | Auditor suite + smoke (1 task per suite, 1 seed) |
| Nightly | Core suites, 1 seed |
| Release | Full suites, ≥ 3 seeds |
| Model upgrade | Full suites + skills library re-evaluation before the new model becomes the default |

**Cost controls** (power preserved):
- paired tasks with common random numbers across arms;
- sequential stopping with alpha spending for A/B comparisons;
- Batch API for single-turn grading;
- exposure-gated skill leave-one-out;
- a monthly eval budget in config (`evals.budget_usd_month`).

## 9. Harness evolution loop

Harness components (system core, CLAUDE.md template, hooks, sub-agent briefs, tool
descriptions, skills) are versioned files. Every change PR states a **predicted effect**
on named metrics. The next suite run confirms or refutes it, and refuted changes are
reverted. Transcripts are distilled into `evals/evidence/`, summaries indexed by failure
mode.

## 10. Acceptance (Set L skeleton)

1. The `noise` and `planted_edge` suites run end to end on the new runtime and publish
   FDR and power.
2. The auditor suite runs in CI with 100% catch and 0 false rejections.
3. A deliberately harmful change (removing the Critic sub-agent) shows as a scorecard
   regression.
4. The `robustness` suite's restart task passes with no duplicate trials.

## 11. Traceability

Implements BS-007 AE-01…AE-07, and CX-13 (non-inferiority). Supports SK-16/SK-17 and B-R6.
