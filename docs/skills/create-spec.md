# Skill: Create Spec

## Purpose
Guide the creation of a specification file that is precise enough to implement without ambiguity and verify without interpretation.

## When to Use
- Defining user-facing behavior for a new feature
- Specifying the interface and contract of a component before building it
- Documenting a data model that will be shared across services
- Describing integration behavior with an external system

---

## Procedures Used
- `procedures/add-spec.md`

---

## Workflow

### 1. Confirm prerequisites
Before writing a spec, verify:
- [ ] **Internet research is done** (Research-First Protocol) — you understand prevailing practice,
      existing building blocks, and the trade-offs. Never spec from assumption.
- [ ] Relevant research briefs exist in `research/`
- [ ] Trade-offs are logged in `docs/open-questions.md`
- [ ] The artifact's success conditions are accessible (`docs/artifact.md`)
- [ ] Any upstream ADRs that constrain this design are written

If research is missing, run `skills/create-research-brief.md` first.

### 2. Identify the spec type
| You are specifying... | Use type |
|-----------------------|----------|
| What a user can do | Feature |
| What a service/module does internally | Component |
| What data looks like | Data |
| How to talk to an external system | Integration |
| The map of how many specs fit together | System-overview |

### 3. Draft the non-goals first
Before filling in behavior, write the **Non-Goals** section. This prevents scope from expanding during writing and surfaces hidden assumptions.

### 4. Execute `procedures/add-spec.md`
Follow the procedure, paying special attention to:
- Acceptance criteria must be **observable and testable** — not "it should feel fast" but "P95 response time is under 200ms"
- Edge cases and error states must be explicitly documented — implementation will hit them regardless
- The spec should be implementable by someone who has never spoken to you

### 5. Review against the artifact's success conditions
After writing the spec, check each success condition in `docs/artifact.md`:
- Is it covered by acceptance criteria in this or another spec?
- If not, either add acceptance criteria or create a new spec for the gap.

### 6. Identify ADRs triggered
Read through your spec and flag every design choice where you had to pick between alternatives. For each significant one, run `skills/create-adr.md`.

---

## Tips

- A spec that says "handle errors gracefully" is not a spec. Name the errors and describe the exact handling.
- Non-goals are as important as goals. They prevent reimplementation of adjacent features and keep scope bounded.
- Acceptance criteria should be written so a QA engineer (or an automated test) can verify them without asking the author.
- If you can't write acceptance criteria for a behavior, you don't understand the behavior yet — do more research or break it down further.
