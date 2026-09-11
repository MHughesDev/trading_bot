# BS-007 / 16 — Agent evaluation suite

**Spec status:** spec-ready draft · **Req prefix:** AE- · **Formal spec:** AGENT-004 ·
**Plan set:** L (skeleton), then extended every phase · **Decisions:** D-06, D-11 ·
**Evidence:** research/R1 §7

---

## 1. Purpose

"Best possible agent" must be measurable. The suite measures the scorecard in 02 §7 on
every change to the harness, prompt, tools, skills or model. It is the release gate for:
- efficiency optimisations (04 §10);
- skill admissions and retirements (13 §10);
- harness evolution (§8).

## 2. Task suites

All tasks run on `synthetic` venue instruments (06 §8) unless marked real.

| Suite | Construction | Grader | Metric |
|---|---|---|---|
| **Planted edge** | GARCH-t noise plus a known signal (AR(1) φ=0.05 at 1h; hour-of-day drift; vol-breakout drift; carry-like drift) at several strengths | Code: did a candidate for the planted mechanism pass G3? | Power curve (pass@k) |
| **Pure noise** | The same generators with no signal | Code: reports "no robust edge" | FDR (**pass^k**) ≤ nominal + 2 pp |
| **Leakage traps** | Look-ahead feature; overlapping labels; model trained into the test window; revised-data use; costs off; **collider** filter; **survivorship** universe; **post-hoc** hypothesis; **complexity-as-vol-timing** | Code: flagged by the agent or Gate 0 | Catch rate 100% |
| **Q&A accuracy** | Questions with computed ground truth ("30-day RV of X", "largest drawdown in 2025", "does X mean-revert after 3σ drops, vs matched random entries?"); some on real data inside a fixed window | Code: numeric tolerance; model grader for caveats | Accuracy |
| **Modelling** | Synthetic series where a known feature predicts the target at a known OOS R² | Code: agent's OOS R² and CW result vs truth; baseline comparison present | Accuracy and honesty |
| **Robustness** | Injected tool failures, timeouts, worker loss, a platform restart mid-session | Code: completes or resumes; no duplicate trials | Pass rate |
| **Efficiency-specific** | *Compaction amnesia* (ids and verdicts planted early, recalled after 2 compactions); *duplicate trials* (no re-run after compaction or resume); *re-fetch rate* | Code | Pass; rate |
| **Report quality** | All suites | Model grader (Batch) plus monthly human review of a transcript sample | Rubric score |

## 3. Auditor planted-violation suite (Gate 0 in CI)

A deterministic CI suite of known violations run directly against Gate 0 and the
validators, without the agent:
- look-ahead features;
- `bfill` and centred windows (truncation test);
- overlapping labels without purge;
- prediction series with training-window overlap;
- revised-data use;
- costs off;
- grade-D data;
- uncited or mismatched report numbers.

**Target:** 100% caught, 0 false rejections on the clean twins (research/R1 §4.1).

## 4. Metrics

- **Scorecard (02 §7):**
  - power;
  - FDR;
  - uncited-number rate;
  - Q&A accuracy;
  - trap catch rate;
  - agent vs random-search arm;
  - robustness.
- **Efficiency:**
  - cost per correct verdict (dollars, trials, wall time);
  - cache hit rate;
  - tokens and tool calls per task;
  - compactions per 100 turns.
- **Skills:** trigger precision and recall, and leave-one-out contribution (13 §10).

## 5. Protocol

- **Seeds:** ≥ 3 per task, with fresh synthetic seeds each run.
- **Held-out tasks:** 30–40% of tasks are held out, and never used for tuning prompts,
  descriptions or skills.
- **Reporting:** pass@k for discovery tasks; pass^k for "never reports an edge on noise"
  and for honesty rules.
- **Settings:** production model and effort, always.
- **Read the transcripts:** failures are triaged for grader bugs before being counted.

## 6. Non-inferiority for optimisations

Every efficiency or cost change (04) ships alone and must pass, on pre-registered
margins:
- FDR ≤ nominal + 2 pp;
- power within −2 pp (95% CI);
- Q&A non-inferior;
- the same trap catch rate;
- uncited rate 0;
- amnesia and duplicate-trial tests passing.

## 7. Cadence and cost control

| Cadence | Scope |
|---|---|
| Per PR touching the harness, tools or prompts | Auditor suite (§3) plus a small smoke set |
| Nightly | Core suites at 1 seed |
| Weekly / release | Full suites at ≥ 3 seeds |
| Model upgrade | Full suites plus the full skills library re-evaluation, before the new model becomes the default |

**Cost control** (no loss of statistical power):
- paired designs with common random numbers;
- sequential stopping with alpha spending;
- Batch for single-turn grading;
- leave-one-out only for skills with enough exposure;
- a monthly eval budget (00 cross-cutting question 4).

## 8. Harness evolution loop

The model is held fixed while the harness evolves: prompts, CLAUDE.md, tool descriptions,
hooks, skills and sub-agent briefs. The loop (Agentic Harness Engineering):
1. Every harness component is a versioned file.
2. Transcripts are distilled into an evidence corpus.
3. Each edit ships with a **predicted effect**, e.g. "the tri-alignment check cuts wasted
   backtests by ≥ 10%". The edit is kept only if the next suite run confirms the
   prediction, with no scorecard regression.

## 9. Requirements

| ID | Requirement | Pri |
|---|---|---|
| AE-01 | The §2 suites MUST exist, using the synthetic venue | MUST |
| AE-02 | The auditor suite (§3) MUST run in CI on every relevant PR, with 100% catch and 0 false rejections | MUST |
| AE-03 | The protocol (§5) MUST hold: seeds, held-out tasks, pass@k/pass^k, production settings | MUST |
| AE-04 | Non-inferiority margins (§6) MUST gate every efficiency change | MUST |
| AE-05 | The scorecard MUST be published per harness version | MUST |
| AE-06 | The model-upgrade cadence MUST include the skills library re-evaluation | MUST |
| AE-07 | Harness edits SHOULD follow the §8 predicted-effect loop | SHOULD |

## 10. Acceptance criteria

1. The Phase 1 skeleton runs the pure-noise and planted-edge suites end to end against
   the new runtime, and publishes FDR and power.
2. The auditor suite catches every planted violation in CI.
3. A deliberately harmful harness change (for example, removing the Critic) is caught by
   the suite as a scorecard regression.
