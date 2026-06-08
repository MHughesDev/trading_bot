# Procedure: Add Architecture Decision Record

## Purpose
Create a new, properly numbered ADR file and register it in the ADR index.

## Trigger
Use this procedure when a significant architectural or technology decision needs to be recorded — whether the decision is proposed, just made, or being revisited.

## Inputs
Before starting, gather:
- A short title for the decision (e.g., "use PostgreSQL for primary datastore")
- The context: what forces, constraints, or requirements led here?
- The decision that was made (or is being proposed)
- At least two alternatives that were considered and why they were not chosen
- The known consequences (positive and negative)

---

## Steps

### 1. Determine the next ADR number
1. List all files in `adr/` that match the pattern `NNNN-*.md`.
2. Find the highest number currently used.
3. The new ADR number is that number + 1, zero-padded to 4 digits (e.g., `0001`, `0012`).
4. If no ADRs exist yet, start at `0001`.

### 2. Create the ADR file
1. Create a new file: `adr/NNNN-short-title-in-kebab-case.md`
   - Replace `NNNN` with the number from step 1.
   - Keep the title short (3–6 words), lowercase, hyphenated.
   - Example: `adr/0003-adopt-event-sourcing.md`

### 3. Fill in the ADR content
Use this template exactly:

```markdown
# ADR-NNNN: [Title in Title Case]

**Status:** Proposed
**Date:** YYYY-MM-DD
**Deciders:** [names or roles]

## Context

[What situation, constraint, or requirement forced a decision?
What options were on the table? What did you not want to decide?]

## Decision

[State the decision clearly and directly in one or two sentences.]

## Rationale

[Why this option over the others? What trade-offs were consciously accepted?]

## Consequences

**Positive:**
- [What becomes easier or better?]

**Negative:**
- [What becomes harder or worse?]

**Neutral:**
- [What changes without clear valence?]

## Alternatives Considered

### Option A: [Name]
[Description and reason not chosen]

### Option B: [Name]
[Description and reason not chosen]

## References

- [Link to relevant research, specs, or external sources]
```

### 4. Update the ADR index
1. Open `adr/README.md`.
2. Add a row to the Index table:
   ```
   | NNNN | [Title] | Proposed | YYYY-MM-DD |
   ```
3. Keep the table sorted by ADR number ascending.

### 5. Cross-reference from related files and trigger re-derivation
- If a spec was the trigger for this ADR, add a link to the ADR in that spec file.
- If this ADR supersedes an earlier one, update the earlier ADR's status line:
  ```
  **Status:** Superseded by ADR-NNNN
  ```
  And update the index entry for the old ADR.
- **Re-derivation trigger:** If this ADR is `Accepted` and changes a constraint or decision that
  affects how planned work is done, open `plans/README.md` and find every formal plan whose
  **Derived From** section lists this ADR. For each such plan, open the plan file and set:
  ```
  **Derivation Status:** STALE — [ADR-NNNN] accepted YYYY-MM-DD, re-derive before next execution
  ```

---

## Outputs
- A new file: `adr/NNNN-title.md`
- Updated index table in `adr/README.md`

## Checklist
- [ ] ADR number is unique and sequential
- [ ] File name matches `NNNN-kebab-case-title.md`
- [ ] All template sections are filled (no placeholder text remaining)
- [ ] Status is set correctly
- [ ] Date is accurate
- [ ] At least two alternatives are documented
- [ ] Index row added to `adr/README.md`
- [ ] Any superseded ADR updated with new status
- [ ] (If Accepted and affects planned work) Any plan listing this ADR in Derived From is marked `STALE`

## Related
- Skill: `skills/create-adr.md`
- Procedure: `procedures/add-spec.md` (specs often trigger ADRs)
