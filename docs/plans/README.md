# plans/

Formal plans for the Python → Rust trading platform refactor. Each plan file is a self-contained, executable document with a stable task-ID structure, acceptance criteria, and phase exit criteria.

> **Execution order is mandatory.** Phases run A → B → 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7, with the single allowed parallelism being Phase 4 starting once Phase 2 is done (Phase 3 not required). See the master plan for sequencing rules and decision gates.
>
> **Two copies of the plans exist** — `refactor_reference_docs/plans/` (canonical executable) and this directory (documentation record with Derived From traceability). If they appear to disagree on technical content, `refactor_reference_docs/plans/` wins.

---

## What Belongs Here

- Formal phase plans and the master plan
- The execution record linking each plan to the specs and ADRs it implements

## What Does Not Belong Here

- Architecture decisions — record those in `adr/`
- Specifications — record those in `specs/`
- Research that informed the plans — record that in `research/`

---

## Index

| Plan | Type | Status | Derived From |
|------|------|--------|--------------|
| [rust-rewrite-master-plan.md](./rust-rewrite-master-plan.md) | Formal | Current | SYS-001, all ADRs |
| [phase-A-documentation.md](./phase-A-documentation.md) | Formal | Current | All specs/ADRs |
| [phase-B-bootstrap.md](./phase-B-bootstrap.md) | Formal | Pending | SYS-001, ADR-0001 |
| [phase-0-foundations.md](./phase-0-foundations.md) | Formal | Pending | DATA-001, DATA-002, DATA-003, DATA-004, ADR-0002, ADR-0007 |
| [phase-1-spine.md](./phase-1-spine.md) | Formal | Pending | COMP-001, COMP-004, ADR-0003, ADR-0006 |
| [phase-2-money-safety.md](./phase-2-money-safety.md) | Formal | Pending | COMP-002, ADR-0005, ADR-0006 |
| [phase-3-ui-streaming.md](./phase-3-ui-streaming.md) | Formal | Pending | COMP-003, ADR-0011 |
| [phase-4-strategies.md](./phase-4-strategies.md) | Formal | Pending | FEAT-001, DATA-004, ADR-0007, ADR-0008 |
| [phase-5-front-doors.md](./phase-5-front-doors.md) | Formal | Pending | DATA-004, INTG-001, ADR-0010 |
| [phase-6-second-asset.md](./phase-6-second-asset.md) | Formal | Pending | COMP-001, COMP-002, ADR-0006 |
| [phase-7-cutover.md](./phase-7-cutover.md) | Formal | Pending | All specs, all ADRs |
