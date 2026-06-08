# Skill: Generate System Design

## Purpose
Transform a user's idea description into a complete system design — spanning research, planning, specifications, and architecture decisions — using a structured, evidence-based workflow.

## When to Use
- A user describes a new idea, product, feature, or application and wants help designing it
- A project needs to go from "rough concept" to "ready to build"
- An existing system needs a redesign or significant extension

---

## Procedures Used
- `procedures/create-artifact.md`
- `procedures/add-research.md`
- `procedures/add-plan.md`
- `procedures/add-spec.md`
- `procedures/add-adr.md`

---

## Workflow

### Phase 0: Fill in the Artifact (MANDATORY — do not skip)
Follow `skills/create-artifact.md`:

1. Open `docs/artifact.md` and fill in all eight sections with the user
2. Confirm with the user before continuing — get an explicit "yes, that's right"
3. Identify research candidates from "What Don't We Know Yet?"

Do not proceed to Phase 1 until the artifact is confirmed.

### Phase 1: Research
For each open question identified in the artifact and for each significant technology choice:
1. Apply `procedures/add-research.md` for each question
2. Prioritize research that addresses the failure modes in the artifact directly
3. After all research is complete, summarize key findings and recommendations for the user before moving on

### Phase 2: Plan
> Plans are derived, never assumed. If you cannot yet derive a milestone from a spec, decision, or
> success condition, you are not ready to plan that part — return to research or specs first.
1. Apply `procedures/add-plan.md` to create a Formal plan in `plans/`
2. Derive the plan from existing artifacts — record them in the plan's **Derived From** section:
   - Milestones map to the artifact's success conditions and the specs that satisfy them
   - Build order follows the architecture's component dependencies
   - Each task traces to a spec's acceptance criteria
   - Risks map to the artifact's failure modes and open questions
3. Present the plan to the user for review before moving to specs

> Note on ordering: a first pass often plans before every spec exists. That is fine for a working
> plan, but a formal plan must name the specs it sequences — write the missing specs, then plan.

### Phase 3: Specify
For each component or feature identified in the plan:
1. Apply `procedures/add-spec.md`
2. Order: start with the most foundational components (data models, core services) before dependent features
3. Each spec's acceptance criteria should map to one or more success conditions from the artifact
4. If the system is complex enough to warrant a unified reference, create a system-overview spec that links all other specs and includes a system diagram (ASCII or description)

### Phase 4: Decide
For each significant design choice made during spec writing:
1. Apply `procedures/add-adr.md`
2. An ADR is warranted when:
   - Two or more reasonable alternatives existed
   - The choice has long-term consequences
   - A future maintainer would reasonably wonder "why was this done this way?"
3. Link each ADR back to the spec(s) that prompted it

### Phase 5: Verify
Before considering the design session done:
1. Run `skills/verify-traceability.md` across the whole `docs/` tree.
2. Fix hygiene failures directly; surface any uncovered acceptance criteria, stale plans, or
   undefined IDs to the user — these usually mean upstream work is missing, not a typo.
3. Run `skills/trace-uncertainty.md` to render the risk surface, and show the user how much of the
   design still rests on open questions or unvalidated assumptions before they commit to building.
4. Re-run until the report is clean (or the only remaining items are intentional and acknowledged).

---

## Output Checklist

At the end of a full system design session, the following should exist:

- [ ] `docs/artifact.md` is filled in and confirmed
- [ ] At least one research brief per major technology choice or open question
- [ ] A Formal plan in `plans/` with milestones linked to the artifact's success conditions
- [ ] Specs for each major component and user-facing feature
- [ ] ADRs for each significant architectural decision
- [ ] All folder READMEs updated with new index entries
- [ ] `verify-traceability` run and clean (Phase 5)

---

## Tips

- The artifact is the contract. A spec written before the artifact is confirmed will solve the wrong problem precisely.
- Failure modes are as important as success conditions. A design that only optimizes for the happy path will fail in production.
- Do not write specs before research is done — specs written in ignorance require rewrites.
- ADRs written after the fact ("we already decided this, just document it") are better than no ADRs, but the best ADRs are written while the decision is live.
- When in doubt about whether something warrants an ADR: if you would want to know *why* it was done that way in 6 months, write the ADR.
