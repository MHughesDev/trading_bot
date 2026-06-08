# Procedure: Execute Plan

## Purpose
Execute a formal plan milestone by milestone and task by task, maintaining task status throughout
so the plan is always an accurate live record of what is in progress, complete, or blocked.

## Trigger
Use this procedure when:
- An agent is beginning work on a milestone in a formal plan
- Resuming work on a plan after a pause or context switch
- Picking up a task another agent or session left in progress

## Inputs
Before starting:
- The formal plan file in `plans/` with all tasks in `NOT STARTED` status
- `Derivation Status` is `Current` — if it says `STALE`, re-derive the plan first (see `add-plan.md`)
- Confirm the milestone's upstream dependencies are met

---

## Steps

### 1. Check derivation status and traceability
Open the plan file. Read the `Derivation Status` header field.
- If `Current` — proceed.
- If `STALE` — stop. Re-run `procedures/add-plan.md` to re-derive the plan against the changed
  artifacts before executing any tasks. Do not execute a stale plan.

Then run `procedures/verify-traceability.md`. If it reports any reference that breaks traceability
(an unresolved `Q-`, `SC-`, `§`, or a broken link in this plan), fix it before executing — an agent
must not implement against a plan whose references do not resolve.

### 2. Confirm upstream milestone dependencies are met
Before beginning a milestone, verify that every milestone it depends on (listed in the
dependency-ordered sequence) is fully `COMPLETE` or that its incomplete tasks are all
`BLOCKED:UPSTREAM`, `BLOCKED:HUMAN`, or `BLOCKED:TESTING`.
If a dependency is not met, do not begin this milestone — surface the blocker to the user.

### 3. Work tasks one at a time, updating status twice per task

For each task in the milestone, follow this sequence exactly:

**Step 3a — Mark IN PROGRESS before touching anything:**
Update the task's status label in the plan file from `NOT STARTED` to `IN PROGRESS`.
Do this before writing a single line of code, config, or documentation for the task.

**Step 3b — Complete the task end-to-end:**
Do the full work required by the task. A task is complete when its corresponding spec acceptance
criterion (cited in the task's source trace) is satisfied end-to-end — not partially.

**Step 3c — Record verification evidence on the acceptance criterion:**
A task is verified, not just done. Open the spec the task traces to and, on the acceptance
criterion it satisfies (§6.1.x), replace `Verified by: [—]` with the evidence — a test name, a
manual-test result + date, or a link — and tick the checkbox. This closes the chain
requirement → criterion → evidence. If the criterion can only be confirmed by physical testing,
leave the task `BLOCKED:TESTING` rather than ticking the box on unverified work.

**Step 3d — Mark COMPLETE before moving on:**
Update the task's status label from `IN PROGRESS` to `COMPLETE`.
Do this before beginning the next task.

> **The rule is strict:** never move to the next task while the current one is `IN PROGRESS`.
> The only valid states for moving on are `COMPLETE` or one of the three blocked states below.

### 4. The only acceptable blocked states

A task may be left in a non-`COMPLETE` state only for these three reasons:

| Status | Condition | How to handle |
|--------|-----------|---------------|
| `BLOCKED:UPSTREAM` | The task depends on work in a future milestone or a task not yet complete | Mark blocked, note what it is waiting on, move to the next unblocked task |
| `BLOCKED:HUMAN` | The task requires action only a human can take (e.g., rotating production keys, provisioning credentials, approving a change) | Mark blocked, leave a clear note of exactly what the human must do |
| `BLOCKED:TESTING` | The task requires physical testing of the running application (e.g., verifying a payment flow end-to-end in a live environment) | Mark blocked, note what test is needed and what environment |

Any other reason to leave a task incomplete is not acceptable. If implementation is hard, do the
work. If the task is underspecified, update the spec first (and re-derive the plan if needed), then
return and complete the task.

### 5. After each milestone

Once every task in a milestone is either `COMPLETE` or blocked with a valid reason:
1. Summarize the milestone's outcome to the user.
2. List any `BLOCKED:*` tasks, what they are waiting on, and who needs to act.
3. Do not begin the next milestone until the user acknowledges the summary.

### 6. After the full plan is executed

When all milestones are complete (or all remaining tasks are validly blocked):
1. Run the coverage check from `procedures/add-plan.md` in reverse — verify every spec acceptance
   criterion cited in the plan is now `COMPLETE` or explicitly blocked.
2. Update the plan's `Status` header field to `Complete`.
3. Surface any `BLOCKED:*` tasks that still require human or testing action.

---

## Task Status Format Reference

Tasks in a plan use a backtick-prefixed status label as the first element of the line:

```
- `NOT STARTED` Task description (→ SPEC-ID §N.M.X)
- `IN PROGRESS` Task description (→ SPEC-ID §N.M.X)
- `COMPLETE` Task description (→ SPEC-ID §N.M.X)
- `BLOCKED:UPSTREAM` Task description (→ SPEC-ID §N.M.X) — waiting on [task/milestone]
- `BLOCKED:HUMAN` Task description (→ SPEC-ID §N.M.X) — [exact action required]
- `BLOCKED:TESTING` Task description (→ SPEC-ID §N.M.X) — [test required and environment]
```

---

## Outputs
- The plan file updated with current task statuses throughout
- A milestone summary communicated to the user after each milestone
- `Status: Complete` set on the plan when execution finishes

## Checklist
- [ ] Derivation status confirmed `Current` before execution began
- [ ] Every task was marked `IN PROGRESS` before work started on it
- [ ] Every task was marked `COMPLETE` (or a valid blocked state) before moving on
- [ ] No task was left `IN PROGRESS` at end of session without explicit reason
- [ ] Milestone summary given to user before beginning the next milestone
- [ ] All `BLOCKED:*` tasks have a clear note stating what is needed and from whom

## Related
- Skill: `skills/execute-plan.md`
- Procedure: `procedures/add-plan.md` (plan creation, coverage check, and re-derivation)
