# Skill: Create ADR

## Purpose
Guide the creation of a well-formed Architecture Decision Record that captures context, decision, rationale, and consequences.

## When to Use
- A significant technology or architecture choice is being made
- Two or more reasonable alternatives were considered
- A future maintainer would reasonably ask "why was this done this way?"
- A previous decision is being revisited or reversed

---

## Procedures Used
- `procedures/add-adr.md`

---

## Workflow

### 1. Determine if an ADR is warranted
Ask:
- Does this decision have long-term architectural consequences?
- Were there real alternatives considered?
- Would a team member six months from now wonder why this was chosen?

If all three are yes, proceed. If the answer is "this was obvious" or "there was only one option," consider a doc comment or inline note instead.

### 2. Gather context before writing
Before opening a file, collect:
- What situation forced this decision?
- What options were considered (at least two)? — these should already be researched and logged in
  `docs/open-questions.md`; an ADR records a decision, it does not substitute for the research behind it
- What are the known trade-offs of the chosen option?
- Who was involved in making this decision?

### 3. Execute `procedures/add-adr.md`
Follow the procedure exactly, including:
- Assigning the correct sequential number
- Using the exact template
- Updating the index

### 4. Set the correct status
| Situation | Status to set |
|-----------|---------------|
| Decision proposed, not yet approved | `Proposed` |
| Decision made and in effect | `Accepted` |
| Decision still valid but not recommended | `Deprecated` |
| Decision replaced by a newer ADR | `Superseded by ADR-NNNN` |

### 5. Link to related artifacts
- Spec that prompted this ADR → add ADR link in spec's Related section
- Research that informed this ADR → add research link in ADR's References
- Open-questions entry this ADR settles → mark it `Resolved` in `docs/open-questions.md`
- If the decision changes the system's shape → update `docs/architecture.md`
- If superseding an old ADR → update the old ADR's status field

---

## Tips

- The **Context** section is the most important part. Decisions look obvious in hindsight; the context explains why it wasn't obvious at the time.
- The **Alternatives Considered** section prevents the team from relitigating already-rejected options.
- Keep ADRs short. If the rationale needs more than a few paragraphs, the decision may need to be decomposed.
- ADRs are immutable historical records. To change a decision, write a new ADR — never edit the original except to update its status field.
