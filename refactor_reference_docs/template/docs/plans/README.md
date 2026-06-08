# plans/

All project plans live here — both lightweight working plans and formal versioned roadmaps.
Use the `Type` field to distinguish them.

---

## Plan Types

| Type | Use when | Lifespan |
|------|----------|----------|
| **Working** | Session scratchpad, spike outline, quick scope draft, open question tracker | Temporary — delete or archive when done |
| **Formal** | Multi-phase roadmap, release plan, migration plan, anything shared with stakeholders | Persistent — versioned across sessions |

A working plan is thinking in progress. A formal plan is a commitment made visible.

**Plans are derived, never assumed.** A formal plan sequences work already established by the
artifact, architecture, specs, ADRs, and research — every milestone and task traces to a source
(see the plan's **Derived From** section). If the work has no source, the gap belongs upstream
(a missing spec, ADR, or research brief), not in the plan.

---

## What Belongs Here

- Quick scope notes for a session or sprint (Working)
- Open question trackers before committing to a design (Working)
- Project roadmaps with milestones (Formal)
- Release plans with defined success signals (Formal)
- Multi-phase technical migration plans (Formal)

## What Does Not Belong Here

- Architecture decisions → `adr/`
- Feature specifications → `specs/`
- Research findings → `research/`
- The foundational project definition → `docs/artifact.md`

---

## Conventions

- File names: `kebab-case.md` — date-prefix working plans if useful: `2024-01-15-auth-spike.md`
- Every plan includes a `Type:` field in its header (Working | Formal)
- Formal plans reference prior research and link to relevant ADRs
- Working plans may be deleted once complete; formal plans use status `Complete` or `Superseded`

---

## Index

| File | Type | Description | Status |
|------|------|-------------|--------|
| [example-task-management-app.md](./example-task-management-app.md) | Formal | Example formal plan — delete after reading | Example |
