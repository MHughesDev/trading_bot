# Procedure: Create the Artifact

## Purpose
Fill in `docs/artifact.md` to establish the foundational project definition before any design work begins.

## Trigger
Use this procedure when:
- A user describes a new idea, application, or system
- A project is being scoped before research or planning begins
- An existing project needs its foundational assumptions re-examined

## Inputs
Before starting, gather (or ask for):
- What the user is trying to build
- Who will use it
- Any known constraints (timeline, stack, team, budget)

---

## Steps

### 1. Open `docs/artifact.md`
This file is the template. Fill it in for this project — do not rename or move it.

### 2. Fill in each section in order

Work through the sections top to bottom. If the user has not given you enough to fill a section,
ask one focused question to get what you need. Do not skip sections — a blank section is a gap
in understanding, not a placeholder.

| Section | What to write | What it feeds |
|---------|---------------|---------------|
| What Are We Building? | Name + one-sentence description | Framing for all artifacts |
| Who Uses It? | Primary and secondary users | Feature specs, UX decisions |
| What Problem Does It Actually Solve? | The underlying need, not features | Validates scope, surfaces ADR context |
| What Does Good Look Like? | 3–5 observable success conditions, each numbered `SC-N` | Acceptance criteria in specs, plan milestones (cited as SC-N) |
| What Would Make This Fail? | Likely failure modes, each numbered `FM-N` | Plan risks, research questions (cited as FM-N) |
| What Don't We Know Yet? | Open questions | Research brief candidates → `open-questions.md` (as Q-N) |
| What Are We Deliberately Not Building? | Explicit out-of-scope items | Spec non-goals, plan scope |
| What Are We Working Within? | Constraints that shape every decision | ADR context, plan dependencies |

### 3. Read it back to the user
Summarize what you filled in. Ask: "Does this capture what you're building?" Correct any section
that is wrong before continuing.

### 4. Derive research candidates
From "What Don't We Know Yet?" — identify which open questions need a research brief before
specs or plans can be written. List these for the user.

### 5. Do not proceed to plans or specs until the artifact is confirmed
If a section is genuinely unanswerable right now, note it as `[unknown — revisit]` and flag it
as a risk. But do not skip the step.

---

## Outputs
- A filled-in `docs/artifact.md`
- A list of research candidates from open questions
- User confirmation that the artifact is accurate

## Checklist
- [ ] All eight sections have content (no placeholder text remaining)
- [ ] Success conditions are numbered `SC-N` and observable — a stranger could verify them
- [ ] Failure modes are numbered `FM-N` and specific, not generic ("slow" is not a failure mode; "P95 latency exceeds 500ms under 100 concurrent users" is)
- [ ] Out-of-scope items are explicitly named
- [ ] User has confirmed the artifact is accurate before any downstream work begins

## Related
- Skill: `skills/create-artifact.md`
- Procedure: `procedures/add-research.md` (open questions → research briefs)
- Procedure: `procedures/add-plan.md` (artifact success conditions → plan milestones)
- Procedure: `procedures/add-spec.md` (artifact users + problem → feature specs)
