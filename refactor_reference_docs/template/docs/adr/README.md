# Architecture Decision Records (ADRs)

This folder contains the project's Architecture Decision Records — a chronological log of significant technical decisions, the context that drove them, and the consequences of each choice.

> To add a new ADR, follow the skill: `skills/create-adr.md` and procedure: `procedures/add-adr.md`.

---

## What Is an ADR?

An ADR documents a single architectural decision. It captures:
- The **context** (forces, constraints, options considered)
- The **decision** that was made
- The **status** of the decision (proposed, accepted, deprecated, superseded)
- The **consequences** (what becomes easier, harder, or different)

ADRs are immutable historical records. When a decision changes, a new ADR is created that supersedes the old one — the old ADR is updated only to reflect its new status.

---

## Index

| # | Title | Status | Date |
|---|-------|--------|------|
| 0001 | [Use Clerk for Authentication](./0001-example-use-clerk-for-auth.md) | Accepted | 2024-03-08 |

---

## Naming Convention

```
adr/NNNN-short-descriptive-title.md
```

Examples:
- `adr/0001-use-postgresql-for-primary-datastore.md`
- `adr/0002-adopt-event-driven-architecture.md`
- `adr/0003-replace-rest-with-graphql.md`

---

## Status Values

| Status | Meaning |
|--------|---------|
| `Proposed` | Under discussion, not yet accepted |
| `Accepted` | Decision is in effect |
| `Deprecated` | No longer recommended but not replaced |
| `Superseded by ADR-NNNN` | Replaced by a newer decision |

---

## ADR Template

```markdown
# ADR-NNNN: [Title]

**Status:** Proposed | Accepted | Deprecated | Superseded by ADR-XXXX
**Date:** YYYY-MM-DD
**Deciders:** [names or roles]

## Context

[What forces, constraints, or requirements led to this decision? What options were considered?]

## Decision

[What was decided? State it clearly and directly.]

## Rationale

[Why was this option chosen over the alternatives? What trade-offs were accepted?]

## Consequences

**Positive:**
- [What becomes easier or better?]

**Negative:**
- [What becomes harder or worse?]

**Neutral:**
- [What changes without clear valence?]

## Alternatives Considered

### Option A: [Name]
[Brief description and why it was not chosen]

### Option B: [Name]
[Brief description and why it was not chosen]

## References

- [Links to relevant research, specs, or external resources]
```
