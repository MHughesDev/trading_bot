# Skill: Execute Plan

## Purpose
Guide an agent through executing a formal plan correctly — maintaining task status, respecting
dependency order, and surfacing blockers — so the plan is always an honest live record.

## When to Use
- Before beginning any implementation work on a planned milestone
- When resuming a plan after a context switch or new session
- When picking up a milestone another agent started

---

## Procedures Used
- `procedures/execute-plan.md`

---

## Workflow

### 1. Always read the plan first
Before touching any code, config, or document — open the plan file. Read:
- `Derivation Status` — if `STALE`, stop and re-derive before proceeding
- The full milestone list and its dependency order
- The status of every task — resume from the first `NOT STARTED` or `IN PROGRESS` task

### 2. Never assume status — read it
Do not assume a task is complete because the work looks done. Read the status label. If a previous
session left a task `IN PROGRESS`, treat it as incomplete until you have verified the work and can
mark it `COMPLETE` yourself.

### 3. Execute via `procedures/execute-plan.md`
The procedure is the authority. Follow it precisely, especially:
- **Update to `IN PROGRESS` before any work** — this is not optional and is never retroactive
- **Verify the full acceptance criterion before marking `COMPLETE`** — partial work is not complete
- **Only three reasons to block** — any other reason means finish the task

### 4. When blocked, be specific
A `BLOCKED:*` label with no explanation is not acceptable. Write exactly:
- `BLOCKED:UPSTREAM` — what specific task or milestone it is waiting on
- `BLOCKED:HUMAN` — the exact action the human must take (not "needs config" — "rotate `STRIPE_SECRET_KEY` in the production environment and update `.env.production`")
- `BLOCKED:TESTING` — the exact test to run, the environment needed, and what constitutes pass/fail

### 5. Milestone boundaries require acknowledgment
After completing a milestone, summarize to the user before starting the next. The summary must
include: what was completed, what is blocked and why, and what comes next.

---

## Tips

- A task left `IN PROGRESS` at the end of a session is a trap for the next session. Either
  complete it or block it with a reason before stopping.
- If a task turns out to require more work than expected, do the work — do not split it into a
  new task mid-execution. If the spec was wrong, fix the spec, re-derive the plan, and then execute.
- If you discover that a task's source spec has a gap or contradiction, mark the task
  `BLOCKED:UPSTREAM` (waiting on spec correction), fix the spec, trigger re-derivation, then resume.
- The plan file is the ground truth for execution state. If the plan and the code disagree, the
  plan wins — reconcile the code, then mark the task `COMPLETE`.
