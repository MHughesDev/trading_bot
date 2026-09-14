# The agent evaluation suite

Implements [AGENT-004](../docs/specs/AGENT-004-agent-evaluation-suite.md).

```
evals/
  suites.yaml        suite -> tasks, seeds, cadence, held-out flags
  gates.yaml         pre-registered non-inferiority margins
  tasks/<suite>/     one YAML per task: setup, budget, prompt, grader
  graders/           how a completed trial is scored
  runner/            trial orchestration and the scorecard
  auditor/           (violating, clean) fixture pairs -- no agent needed
  results/           per-run scorecards
  evidence/          distilled transcripts, indexed by failure mode
```

## What runs today

| Piece | State |
|---|---|
| Auditor suite (AE-02) | **Running in CI.** 27 fixtures, 100% catch, 0 false rejections |
| Synthetic venue (DA-13) | **Working.** Seven generators, verified statistically |
| Graders, scorecard, gate | **Working and tested.** 35 tests, no credential needed |
| Task and suite definitions | **Written.** 24 tasks across six suites |
| Live trials | **Blocked.** Needs a provider credential in the encrypted store |

`python evals/runner/run.py --suite noise --seeds 2 --dry-run` exercises task
resolution, seed derivation, grading and the scorecard without a credential. It
grades every stub trial as a failure, which is correct: a trial that produced no
report is a system that avoided the question, not a system that correctly found
nothing.

## The two numbers

**`noise`** measures whether the agent will claim an edge that is not there.
**`planted_edge`** measures whether it can find one that is. A harness change that
improves either at the other's expense has not improved.

`noise` is scored **pass^k** — right on every seed — and `planted_edge` **pass@k**.
The asymmetry is the point. A process that refuses to claim a false edge two times
in three is not one you can act on: the third run is the one that reaches a human,
and nothing about it looks different from the other two.

## Hidden information

The generator parameters and the planted mechanism live in `synthetic_instruments`
behind the `evals.truth` scope. Migration 0039 refuses that scope on any
project-bound session, so no agent token can hold it. The runner holds it; the
session token holds the seven research scopes and nothing else.

If `GET /api/data/synthetic/{id}/truth` ever succeeds with a session token, the
suite has stopped measuring anything, and the correct response is to stop the run.

## A finding about the margins

AGENT-004 §7 asks for two-percentage-point non-inferiority margins on power and Q&A
accuracy. **The arithmetic does not support that at any cadence we can afford.**

Non-inferiority is decided on an interval around the *difference* between two arms,
over trials paired by common random numbers (§8). With perfect agreement between the
arms — the best case a harmless change can produce — Tango's score interval needs:

| Margin | Paired trials per arm |
|---|---|
| 10 pp | 35 |
| 5 pp | 73 |
| **2 pp** | **189** |

A release run is roughly 75 trials, which decides about 4.9 pp. At $6–8 a trial, a
2 pp gate costs $1,100–1,500 per gated pull request.

`gates.yaml` is therefore set to **5 pp**, with `spec_margin: 0.02` recording the
target. The gate reports `UNDERPOWERED` — distinct from `FAIL` — when a run is too
small to decide its margin, because an underpowered run has not shown a regression,
it has shown nothing, and conflating the two teaches everyone to read gate failures
as noise.

Two ways to reach 2 pp, both decisions about money and scope:

1. **More seeds.** 189 paired trials per arm, per gated PR.
2. **A continuous gated metric** instead of binary pass/fail — an effect size or a
   score rather than "did it reach G3". A continuous measure carries far more
   information per trial, and the required n falls by roughly an order of magnitude.

(2) is the better trade and is not a small change: it means every grader returns a
score as well as a verdict, and the margins are re-expressed in those units.

## Cadence

| Cadence | Scope |
|---|---|
| Per relevant PR | Auditor suite (runs now) + smoke: 1 task per suite, 1 seed |
| Nightly | Core suites, 1 seed |
| Release | Full suites, ≥ 3 seeds |
| Model upgrade | Full suites + skills re-evaluation, before the new model is default |

## Held-out tasks

30–40% of each suite is flagged `held_out` in `suites.yaml`. Held-out tasks are
never used to tune prompts, CLAUDE.md, tool descriptions, skills or skill
descriptions. The flag lives in the suite file rather than the task file so that the
held-out set can be read in one glance by someone checking whether it was respected.

## Triage

Every failure is triaged for grader bugs before it is counted, and the label is
stored. A failure labelled `grader_bug` is excluded from the metric; a failure with
**no** label counts as a failure and blocks the gate. That asymmetry is deliberate:
excluding untriaged failures would make "do not look at it" the cheapest way to
raise a score.
