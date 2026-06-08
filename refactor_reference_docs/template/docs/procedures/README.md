# procedures/

Atomic, step-by-step instructions for every repeatable task in this repo. Procedures are the source of truth. Skills in `skills/` reference and compose procedures — they never replace them.

---

## What Is a Procedure?

A procedure is:
- **Atomic** — one task, one file, one clear outcome
- **Reusable** — written without project-specific names or paths
- **Complete** — someone following it should not need to guess or improvise
- **Agent-agnostic** — works whether executed by a human or an AI

A procedure is not:
- A high-level overview (that belongs in a skill or README)
- A project plan (that belongs in `plans/`)
- A design decision (that belongs in `adr/`)

---

## Procedure Index

| File | Task |
|------|------|
| [create-artifact.md](./create-artifact.md) | Fill in the foundational project definition (run first) |
| [add-research.md](./add-research.md) | Add a new research brief |
| [add-plan.md](./add-plan.md) | Add a new plan (working or formal) |
| [execute-plan.md](./execute-plan.md) | Execute a plan task by task with status tracking |
| [add-spec.md](./add-spec.md) | Add a new specification file |
| [add-adr.md](./add-adr.md) | Add a new Architecture Decision Record |
| [verify-traceability.md](./verify-traceability.md) | Check that all cross-references, coverage, and links are intact |
| [analyze-impact.md](./analyze-impact.md) | Compute the downstream blast radius of a proposed change before making it |
| [explain-provenance.md](./explain-provenance.md) | Walk upward to the causal chain that justifies an element's existence |
| [trace-uncertainty.md](./trace-uncertainty.md) | Render the risk surface — the design resting on unresolved questions and assumptions |

---

## Authoring a New Procedure

When creating a new procedure file, use this structure:

```markdown
# Procedure: [Task Name]

## Purpose
One sentence: what does following this procedure produce?

## Trigger
When should this procedure be used?

## Inputs
What information must be known or gathered before starting?

## Steps
1. Step one
2. Step two
   - Sub-step if needed
3. Step three

## Outputs
What artifact(s) does this procedure produce?

## Checklist
- [ ] Item 1
- [ ] Item 2

## Related
- Skill: `skills/relevant-skill.md`
- Procedure: `procedures/related-procedure.md`
```

---

## Rules for Modifying Procedures

- Never modify a procedure without explaining the change to the user.
- If a procedure is no longer accurate, update it — do not delete it.
- If a task has split into two distinct procedures, create a second file; do not bloat the original.
