# Skill: Create Plan

## Purpose
Create a plan document — working or formal — that sequences work already established by the artifact,
architecture, and specs into executable milestones. **A plan is derived, never assumed.**

## When to Use
- After the artifact, architecture, and the relevant specs exist — a formal plan sequences them
- When scoping a sprint, phase, or session of work
- When a large feature needs to be broken into sequenced steps
- When dependencies and risks need to be made explicit before work starts

> If the specs and decisions a plan would sequence do not exist yet, you are not ready to plan.
> Create the missing spec, ADR, or research brief first — do not plan against assumptions.

---

## Procedures Used
- `procedures/add-plan.md`

---

## Workflow

### 1. Determine plan type
All plans live in `plans/`; the `Type:` field distinguishes them.

| Use `Type: Working` when... | Use `Type: Formal` when... |
|-----------------------------|----------------------------|
| Notes are for this session only | The plan persists across sessions |
| You're thinking through scope | The plan has defined milestones |
| The plan may be discarded | The plan will be reviewed in retrospectives |
| Low formality is fine | Stakeholders will read it |

### 2. Gather the source artifacts (the plan is derived from these)
Before writing a formal plan, read and list what it will sequence:
- [ ] `docs/artifact.md` — success conditions and failure modes
- [ ] `docs/architecture.md` — components and the build order they imply
- [ ] `docs/specs/` — the specs whose acceptance criteria become tasks
- [ ] `docs/adr/` — decisions that constrain how the work is done
- [ ] `docs/research/` and `docs/open-questions.md` — unresolved forks that become risks/dependencies

If a spec, decision, or research brief the plan would depend on is missing, create it first. Do not
fill the gap with an assumption inside the plan.

### 3. Derive milestones and tasks from the sources
Build the plan up from the artifacts — never top-down from intuition:
- Every milestone advances at least one success condition and names the spec(s) it builds
- Every task traces to a spec's acceptance criteria or an architecture component
- The build order follows the architecture's dependencies (foundational components first)
- Every risk traces to a failure mode in the artifact or an open question
- If a milestone or task has no source, either find its source or cut it

### 4. Execute `procedures/add-plan.md`
Follow the procedure. For formal plans, the three mandatory steps beyond the template are:

- **Step 4 — Sequence from real dependencies:** derive milestone order from architecture component
  dependencies and spec `§1.3 Related` links; do not order by intuition
- **Step 5 — Coverage check:** verify every spec AC and every artifact success condition has a task
  trace, and every task has a source; resolve all gaps before marking the plan Active
- All tasks begin at `NOT STARTED` status — do not pre-fill any other status

### 5. Identify specs and ADRs triggered by the plan
After writing the plan, identify:
- What components or features need specs written? → run `skills/create-spec.md` for each
- What design decisions are implied by the plan's approach? → run `skills/create-adr.md` for each

### 6. Hand off to execution
Once the plan is complete and passes the coverage check, tell the user it is ready.
When work begins, follow `skills/execute-plan.md` — do not begin a milestone without it.

---

## Tips

- A milestone without a success signal is not a milestone — it is a wish.
- List risks explicitly. A plan that pretends risks don't exist will be ambushed by them.
- Plans should be short enough to read in one sitting. If a plan exceeds ~4 pages, it is covering too much scope and should be split into phases.
- Working plans (`Type: Working`) can be rough. Formal plans (`Type: Formal`) must be precise enough that someone else could execute them.
