# plans-set-B/

Formal execution plans for the **full end-state vision** of the multi-asset trading platform
(Rust monolith + satellite collectors + React frontend). Each plan file is a self-contained,
executable document with a stable task-ID structure, concrete acceptance criteria, and phase
exit criteria. Every file is written to be executable by **Claude Haiku 4.5** with no memory of
prior conversation — only the file itself plus the existing codebase.

> **Read [`master-plan.md`](./master-plan.md) first.** It defines the vision, the invariants every
> phase obeys, the phase sequence, and the START-HERE entry point. Then execute one phase file at a
> time, in order (Phase 1 → 7).

---

## What Belongs Here

- The set-B master plan and its seven phase files
- Self-contained execution plans tracing to research conclusions (`C-NNN`), specs, and ADRs

## What Does Not Belong Here

- Architecture decisions — those live in `docs/adr/`
- Specifications — those live in `docs/specs/`
- Research / brainstorms / open questions that informed these plans — those live in `docs/research/`

---

## Index

| Plan | Type | Status | Derived From |
|------|------|--------|--------------|
| [master-plan.md](./master-plan.md) | Formal | Current | SYS-001, all C-0xx end-state conclusions, all ADRs |
| [phase-1-registry-and-paper-engine.md](./phase-1-registry-and-paper-engine.md) | Formal | Pending | C-055, C-056, C-058, C-068, C-073, C-086, C-087, C-088, C-089, C-090, C-092, C-093, C-105, C-114 |
| [phase-2-collector-infrastructure.md](./phase-2-collector-infrastructure.md) | Formal | Pending | C-017, C-060, C-065, C-077, C-091, C-099, C-101, C-102, C-103, C-106, C-112 |
| [phase-3-strategy-system.md](./phase-3-strategy-system.md) | Formal | Pending | C-061, C-072, C-082, C-084, C-085, C-100, C-113, C-117 |
| [phase-4-execution-and-ledger.md](./phase-4-execution-and-ledger.md) | Formal | Pending | C-017, C-056, C-058, C-059, C-068, C-073, C-086, C-087, C-088, C-092, C-105 |
| [phase-5-frontend-trading-workspace.md](./phase-5-frontend-trading-workspace.md) | Formal | Pending | C-012, C-013, C-026, C-060, C-094, C-096, C-112, C-118, C-119, C-120, C-123, C-126, C-127, C-129 |
| [phase-6-frontend-dashboard-and-automations.md](./phase-6-frontend-dashboard-and-automations.md) | Formal | Pending | C-015, C-053, C-079, C-080, C-081, C-084, C-093, C-095, C-098 |
| [phase-7-graph-social-and-polish.md](./phase-7-graph-social-and-polish.md) | Formal | Pending | C-043, C-065, C-101, C-102, C-103, C-106, C-110 |

**Status legend:** `Pending` = not started · `Current` = active reference · `Done` = exit criteria green.
