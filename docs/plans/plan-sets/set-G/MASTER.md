# Documentation Restructuring — Set G

**Completion: 0% (0 / 11 primary tasks)**

## Overview

Set G consolidates all documentation, plans, and scattered reference material
into a single `docs/` tree. The repo currently has:

- **6 documentation `.md` files at the repo root** (beside `README.md`):
  `AGENT.md`, `NEWCOMERS.md`, `CONCLUSIONS.md`, `COMPLACENCY_LOG.md`,
  `LATENCY_ISSUES_COMPREHENSIVE.md`, `LATENCY_ISSUES_UNIFIED_TABLE.md`
- **A `plans/` folder at root** with 3 completed plan sets (~80+ files across
  set-C, set-D, set-E, plus new set-F and set-G)
- **A `docs/` folder** with 11 subdirectories, a healthy set of ADRs and
  canonical specs, but no index over plans and no awareness of the root `.md`
  files

The goal is a single, navigable `docs/` tree where every piece of
documentation can be found, linked, and maintained.

**Out of scope:** ADR content is frozen (ADR-0007). Canonical spec content is
frozen. Set G moves and indexes; it does not rewrite technical content.

---

## Guiding Constraints

- **`README.md` stays at root.** GitHub convention; it is the project landing
  page and must not move.
- **ADR content is frozen.** `docs/adr/` files are permanent records of
  architecture decisions. Set G does not modify their content.
- **Canonical specs are frozen.** `docs/specs/` content is the authoritative
  reference for ADR-0007/ADR-0010. Set G does not modify spec content.
- **Use `git mv`, not copy+delete.** Preserves history for every moved file.
- **No dead links.** Every internal cross-reference updated after each move.
- **Archive audit before prune.** `docs/archive/` is catalogued in Phase 0
  before any file is deleted (Phase 3). Never delete without the audit.
- **Procedures stay working.** `docs/procedures/add-plan.md` and
  `docs/procedures/add-doc.md` reference paths; update them.

---

## Phase Summary

| Phase | File | Label | Tasks | Completion | Goal |
|-------|------|-------|-------|------------|------|
| 0 | [phase-0.md](phase-0.md) | Content Audit | 1 | 0% | Catalogue docs/archive/ and identify duplicates |
| 1 | [phase-1.md](phase-1.md) | Root File Migration | 3 | 0% | Move 6 root .md files into docs/ |
| 2 | [phase-2.md](phase-2.md) | Plans Migration | 2 | 0% | Move plans/ → docs/plans/; update all cross-refs |
| 3 | [phase-3.md](phase-3.md) | Archive Prune | 1 | 0% | Delete confirmed duplicates from docs/archive/ |
| 4 | [phase-4.md](phase-4.md) | Index & Navigation | 4 | 0% | Update FOLDER_MAP, docs/README.md, procedures, root README |

---

## Item → Phase Map

| # | Item | Phase · Task |
|---|------|--------------|
| 1 | Catalogue docs/archive/ (30+ files) | 0.1 |
| 2 | Move AGENT.md → docs/procedures/ | 1.1 |
| 3 | Move NEWCOMERS.md → docs/ root | 1.2 |
| 4 | Merge/move CONCLUSIONS.md → docs/research/ | 1.3 |
| 5 | Move COMPLACENCY_LOG.md → docs/governance/ | 1.4 |
| 6 | Move LATENCY_ISSUES_*.md → docs/architecture/ | 1.5 |
| 7 | Move plans/ → docs/plans/ (git mv) | 2.1 |
| 8 | Update all cross-references to plans/ paths | 2.2 |
| 9 | Delete duplicate files from docs/archive/ | 3.1 |
| 10 | Update docs/FOLDER_MAP.md with new structure | 4.1 |
| 11 | Update docs/README.md as the central nav hub | 4.2 |
| 12 | Update root README.md to point to docs/ | 4.3 |
| 13 | Update procedures: add-plan.md, add-doc.md | 4.4 |

(Items 10–13 are grouped into 4 tasks in Phase 4.)

---

## Locked Decisions (2026-06-13)

| # | Decision | Locked Choice |
|---|----------|---------------|
| 1 | Root `.md` files | **Move all 6 into docs/** (appropriate subdirectory per file type); `README.md` stays at root |
| 2 | `plans/` location | **Move into `docs/plans/`** — one authoritative docs tree |
| 3 | `docs/archive/` | **Audit and prune** — delete confirmed duplicates, keep unique historical content |
| 4 | Scattered crate/app `.md` files | **None found** — no action needed in `crates/` or `apps/` |

---

## File Placement Decisions

| File | Current Location | New Location | Rationale |
|------|-----------------|--------------|-----------|
| `AGENT.md` | root | `docs/procedures/AGENT.md` | Instructions for AI agents; fits procedures |
| `NEWCOMERS.md` | root | `docs/NEWCOMERS.md` | Onboarding guide; top-level docs/ entry point |
| `CONCLUSIONS.md` | root | merge into `docs/research/CONCLUSIONS.md` | `docs/research/CONCLUSIONS.md` already exists — audit for overlap, merge or replace |
| `COMPLACENCY_LOG.md` | root | `docs/governance/COMPLACENCY_LOG.md` | Tech-debt tracking; fits governance |
| `LATENCY_ISSUES_COMPREHENSIVE.md` | root | `docs/architecture/latency-analysis.md` | Rename to kebab-case; architecture concern |
| `LATENCY_ISSUES_UNIFIED_TABLE.md` | root | `docs/architecture/latency-table.md` | Companion to the above |
| `plans/` | root | `docs/plans/` | Single docs tree |

---

## Progress Log

| Date | Phase | Task | Note |
|------|-------|------|------|
| 2026-06-13 | — | plan | Set G created. 4 decisions locked. 11 tasks across 5 phases. |
