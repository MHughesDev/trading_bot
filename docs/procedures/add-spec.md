# Procedure: Add Specification File

## Purpose
Create a spec file that defines what a feature, component, API, or data model should do — precisely enough to implement and verify without ambiguity.

## Trigger
Use this procedure when:
- Defining the behavior of a new user-facing feature
- Specifying the interface and responsibilities of a system component
- Documenting an API contract before implementation
- Defining a data model with validation rules and constraints
- Specifying an integration with an external system

## Inputs
Before starting, gather:
- The **type** of spec (feature, component, data, integration, system-overview)
- The **name** of the thing being specified
- **Completed research** (see Step 0) — you may not write a spec without it
- Any relevant **research briefs** from `research/` that inform this spec
- Any relevant **ADRs** from `adr/` that constrain design choices
- The relevant **success conditions** from `docs/artifact.md`

---

## Steps

### 0. Research first (MANDATORY)
Per the Research-First Protocol in `AGENT.md`, do not write a spec from assumption. Before drafting:
1. Research how this kind of thing is commonly specified and built.
2. Identify existing APIs, services, or libraries that could satisfy part of the spec.
3. For every design fork, identify the competing options and their trade-offs.
4. Log each consequential trade-off in `docs/open-questions.md`. Resolve the entry here once the
   research settles it, and cite the research brief.

If you cannot research (no internet access), stop and tell the user — do not assume.

### 1. Determine the spec type and its code

| Type | Code | Use when |
|------|------|----------|
| Feature | `FEAT` | Specifying user-visible behavior end-to-end |
| Component | `COMP` | Specifying an internal service or module |
| Data | `DATA` | Specifying a schema, model, or data contract |
| Integration | `INTG` | Specifying behavior with an external system |
| System-overview | `SYS` | A unified map linking many specs for a complex system |

### 2. Assign the spec ID and file name

Spec files follow a fixed, procedurized naming convention — never freeform:

```
<TYPE>-<NNN>-<kebab-name>.md
```

1. Take the **type code** from Step 1 (e.g. `FEAT`).
2. Determine the next **number**: list files matching `<TYPE>-*.md`, find the highest `NNN`
   for that code, add 1, zero-pad to 3 digits. If none exist, start at `001`. Numbering is
   **per type** — `FEAT-001` and `COMP-001` can both exist.
3. Add a short **kebab-name** describing the subject (the thing, not the verb).

The **Spec ID** is `<TYPE>-<NNN>` (e.g. `FEAT-001`). Use it to cite the spec from plans, ADRs, and
the open-questions log — down to the section: `FEAT-001 §3.1.A`.

| Good | Bad |
|------|-----|
| `FEAT-001-user-authentication.md` | `user-login.md` (no ID) |
| `COMP-002-rate-limiter.md` | `rate-limiter-component.md` (old style) |
| `DATA-001-user-profile-schema.md` | `handle-profiles.md` (verb, no ID) |

**Splitting — one spec, one cohesive responsibility:**
- Split into a second spec (its own ID) when a file covers two things that could be implemented or
  tested independently (e.g. a data model *and* the feature that uses it → one `DATA` + one `FEAT`).
- A spec that needs more than ~7 top-level sections to stay coherent is usually two specs.
- When you split, cross-link the spec IDs in their **Related** sections.

### 3. Fill in the spec template

Every spec uses the **same numbered sections**, each naming one aspect of the spec. Address scheme:
`Section N` → `subsection N.M` → `lettered item N.M.X`. Cite anything as `<SPEC-ID> §N.M.X`.

Keep the section numbers fixed. Append new items at the end of their subsection (next letter) so
existing references never shift. Acceptance items cite the requirement they verify.

```markdown
# Spec: <TYPE>-<NNN> — [Name]

**Spec ID:** <TYPE>-<NNN>
**Type:** Feature | Component | Data | Integration | System-overview
**Status:** Draft | Ready for Review | Approved | Implemented | Deprecated | Superseded by <SPEC-ID>
**Date:** YYYY-MM-DD
**Author:** [name or "Agent"]

## 1. Overview
1.1 Purpose — [what this spec covers, in one paragraph]
1.2 Context — [why it exists and where it sits in the system]
1.3 Related artifacts
   1.3.A ADR: [adr/NNNN-title.md](../adr/NNNN-title.md)
   1.3.B Research: [research/file.md](../research/file.md)
   1.3.C Open questions: [open-questions.md](../open-questions.md) — Q-N
   1.3.D Plan: [plans/file.md](../plans/file.md)

## 2. Scope
2.1 Goals
   2.1.A [What this spec is designed to achieve]
2.2 Non-Goals (out of scope)
   2.2.A [What this spec explicitly does not cover]

## 3. Requirements
3.1 Functional requirements
   3.1.A [The system must …]
   3.1.B [The system must …]
3.2 Non-functional requirements
   3.2.A [Performance, security, or operational constraint — stated measurably]

## 4. Interface / Data
> Include only the block that fits the type, numbered under section 4:
> Feature → user stories · Component → inputs/outputs/errors · Data → schema table ·
> Integration → external contract.
4.1 [Type-specific detail]
   4.1.A [Item]

## 5. Behavior
5.1 Happy path
   5.1.A [Step]
5.2 Edge cases
   5.2.A [Edge case]: [expected behavior]
5.3 Error states
   5.3.A [Error condition]: [expected system behavior]

## 6. Acceptance Criteria
6.1 Criteria (each verifies a requirement from §3; record evidence once met)
   6.1.A [ ] [Observable, testable condition] (verifies §3.1.A) — Verified by: [—]
   6.1.B [ ] [Observable, testable condition] (verifies §3.1.B) — Verified by: [—]
   > Leave `Verified by: [—]` until the criterion is actually met, then replace `[—]` with the
   > evidence: a test name, a manual-test result + date, or a link. The checkbox is ticked only
   > when evidence is recorded. This closes the chain: requirement → criterion → evidence.

## 7. Open Questions & Assumptions
7.1 Open questions — tracked centrally in [open-questions.md](../open-questions.md) (Q-N)
7.2 Assumptions (each carries a validation state; unvalidated assumptions are uncertainty sources)
   7.2.A [Thing assumed true that, if wrong, would change this spec] — Validated: [—]
   > Leave `Validated: [—]` until the assumption is confirmed, then replace `[—]` with the
   > evidence (a test, a vendor confirmation, a research brief). Until then the assumption — and
   > everything resting on it — sits on the risk surface (`trace-uncertainty.md`).
```

### 4. Update the specs index
1. Open `specs/README.md`.
2. Add a row to the Index table:
   ```
   | [<TYPE>-<NNN>-name.md](./<TYPE>-<NNN>-name.md) | <SPEC-ID> | [Type] | Draft | [ADR-NNNN] |
   ```

### 5. Link related files and trigger re-derivation
- Link the ADR(s) and research brief(s) that fed this spec (§1.3).
- Mark any open-questions entries this spec resolved or raised.
- If the spec changes the system's shape, update `docs/architecture.md`.
- **Re-derivation trigger:** If this is an update to an existing spec (any section changed),
  open `plans/README.md` and find every formal plan whose **Derived From** section lists this spec.
  For each such plan, open the plan file and update its header field to:
  ```
  **Derivation Status:** STALE — [SPEC-ID] updated YYYY-MM-DD, re-derive before next execution
  ```
  This ensures no agent executes a plan against an outdated spec.
- For a large or far-reaching change, run `procedures/analyze-impact.md` first to find the full
  downstream closure — not just the plans one hop away — before editing.

### 6. Retiring or replacing a spec
Specs are not deleted — like ADRs, they retire so history stays readable.
- **Deprecated:** the feature/component is no longer planned but nothing replaces it. Set
  `**Status:** Deprecated`, add a one-line note at the top of §1 explaining why and when.
- **Superseded:** a new spec replaces this one. Set `**Status:** Superseded by <NEW-SPEC-ID>` and
  add the same line; the new spec references the old one in its §1.3 Related.
- In both cases, fire the re-derivation trigger (Step 5) for any plan listing this spec, and update
  the row in `specs/README.md`. Do not delete the file — a superseded spec is still cited by history.

---

## Outputs
- A new file: `<TYPE>-<NNN>-name.md` with a Spec ID and fixed numbered sections
- Updated index in `specs/README.md`
- Resolved/raised entries in `docs/open-questions.md`
- Cross-links in related ADRs, research, plans, and (if shape changed) `architecture.md`

## Checklist
- [ ] Research was done first; trade-offs logged in `open-questions.md`
- [ ] File name follows `<TYPE>-<NNN>-name.md`; number is the next free one for that type
- [ ] `Spec ID` is set in the header
- [ ] All seven sections are present and numbered; items use the `N.M.X` scheme
- [ ] §2.2 Non-Goals is present and non-empty
- [ ] Acceptance criteria (§6) are observable, testable, cite the requirement they verify, and carry a `Verified by:` field (`[—]` until met)
- [ ] Edge cases and error states (§5) are documented
- [ ] Related ADRs, research, and open questions are linked (§1.3)
- [ ] Index row added to `specs/README.md`
- [ ] (If updating) Any plan listing this spec in Derived From is marked `STALE`
- [ ] No placeholder text remaining in the template

## Related
- Skill: `skills/create-spec.md`
- Procedure: `procedures/add-research.md` (research precedes every spec)
- Procedure: `procedures/add-adr.md` (spec design choices often warrant an ADR)
