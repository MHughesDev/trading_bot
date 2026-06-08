# Skill: Create Artifact

## Purpose
Guide the user through filling in `docs/artifact.md` — the foundational project definition that every downstream artifact (research, plans, specs, ADRs) is built from.

## When to Use
- Any time a user describes an idea they want to design or build
- Before creating research briefs, plans, or specs for a new project
- When an existing project needs its assumptions revisited

---

## Procedures Used
- `procedures/create-artifact.md`

---

## Workflow

### 1. Recognize the trigger
If the user says anything like "I want to build X", "help me design Y", or "I have an idea for Z" —
this skill runs first. No exceptions.

### 2. Do not start designing yet
The artifact is not a formality. A spec or plan written before the artifact is confirmed will
have to be rewritten. Treat the artifact as the contract that makes everything else cheaper.

### 3. Execute `procedures/create-artifact.md`
Work through all eight sections. The only valid reason to leave a section blank is if the answer
is genuinely unknown — in that case, mark it `[unknown — revisit]` and flag it as a risk.

### 4. Confirm with the user
Once filled in, read the artifact back. Get an explicit "yes, that's right" before moving to
research, planning, or specifications.

### 5. Surface research candidates
Identify which entries in "What Don't We Know Yet?" need a research brief before design can proceed.
Offer to run `skills/create-research-brief.md` for each one.

---

## Tips

- "What Does Good Look Like?" is the hardest section to fill well. Push past vague answers.
  "Users are happy" → "A new user completes their first [core action] in under 3 minutes."
- "What Are We Deliberately Not Building?" often reveals scope disagreements between stakeholders.
  It's worth spending extra time here.
- If the user wants to skip the artifact and jump to specs, explain that specs written before the
  artifact is confirmed tend to solve the wrong problem precisely.
- The artifact is a living document for the first session only — once confirmed, treat it as fixed.
  If scope changes later, note the change in the plan, not by editing the artifact.
