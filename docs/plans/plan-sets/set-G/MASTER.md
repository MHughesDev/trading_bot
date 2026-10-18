# Documentation Restructuring — Set G

**Completion: 13 / 13 done (2026-10-03). Remainder: items 1–3 DONE, items 4
and 5 BLOCKED:HUMAN (exact fix and a recommendation recorded below).**

> **Do not execute this set as written.** Like Set F, the `0 / 11` above was a
> bookkeeping failure. Commit **`5febac2` "docs(set-G): restructure documentation
> — root moves, plans consolidation, archive cleanup" (2026-06-13)** executed
> essentially the whole plan on the day it was written, and nobody ticked the
> boxes. `docs/README.md` has said *"Set G (documentation restructuring) is
> complete"* ever since.
>
> **The task count in this MASTER is also wrong.** The phase table says Phase 1
> has 3 tasks; `phase-1.md` contains 5 (`G-1.1 … G-1.5`) and self-reports
> `0 / 5`. The real count is **13**, not 11, and all 13 are audited below.
>
> | Task | Verdict | Evidence |
> |---|---|---|
> | **G-0.1** catalogue `docs/archive/` | **Done** | `docs/archive/AUDIT.md`, dated 2026-06-13: 41 entries classified 22 DUPLICATE / 1 PARTIAL / 17 UNIQUE, each with its live equivalent named |
> | **G-1.1** `AGENT.md` → `docs/procedures/` | **Done** | `docs/procedures/AGENT.md`; true `git mv` in `5febac2`; no root `AGENT.md` remains |
> | **G-1.2** `NEWCOMERS.md` → `docs/` | **Done** | `docs/NEWCOMERS.md`; linked from root `README.md` and `docs/README.md` |
> | **G-1.3** merge `CONCLUSIONS.md` → `docs/research/` | **Done differently** | merged (−237 root / +105 into `docs/research/CONCLUSIONS.md`), the plan's option 2, not a rename. **But a stale second copy survives at the repo-root `research/` directory** — see remainder item 5 |
> | **G-1.4** `COMPLACENCY_LOG.md` → `docs/governance/` | **Done** (missed cross-ref fixed 2026-10-03) | `docs/governance/COMPLACENCY_LOG.md`; its own line 9 still links `plans/set-E/MASTER.md`, which is dead — the "all cross-references updated" criterion is not met |
> | **G-1.5** `LATENCY_ISSUES_*.md` → `docs/architecture/` | **Done** | `docs/architecture/latency-analysis.md`, `docs/architecture/latency-table.md`; kebab-case as specified, 0 content change |
> | **G-2.1** `plans/` → `docs/plans/` | **Done differently** | landed at `docs/plans/plan-sets/set-X/` — one level deeper than planned — with `docs/plans/reference/` and `docs/plans/special-plans/` as siblings. Renames, so history survives. Sets A–L present |
> | **G-2.2** update `plans/` cross-references | **DONE 2026-10-03** — all 8 fixed, plus three move casualties the audit missed: `docs/plans/reference/rust-rewrite-master-plan.md` (links to `./phase-*.md`, `../specs/`, `../architecture.md`, `../open-questions.md`, broken by the move into `reference/`), `set-B/master-plan.md` (`../plans-set-A/`, `../../adr/`) and `set-C/agents/README.md` (15 links to the pre-rename `agent-NN-<slug>.md` names). Link check over all of those files: 0 dead. Was: **OPEN** | done in `add-plan.md`, `add-doc.md`, `docs/README.md`, root `README.md`, `FOLDER_MAP.md`. Still stale in `docs/governance/COMPLACENCY_LOG.md:9`, `docs/procedures/add-adr.md:96`, `add-spec.md:159`, `execute-plan.md:15`, `README.md:17`, `verify-traceability.md:67`, `docs/research/README.md:20`, `docs/specs/FEAT-002-backtesting.md:138` |
> | **G-3.1** prune DUPLICATE archive files | **DONE 2026-10-03** (sub-step 4 finished: `docs/archive/README.md` rebuilt with 0 dead links; the 24 deleted files are listed by name with links to their live replacements). Was: **Done, sub-step 4 missed** | the 22 files `AUDIT.md` marked DUPLICATE were `git rm`'d in `5febac2`; 34 files remain so the "not empty" criterion holds. **`docs/archive/README.md` was never updated and now carries 24 dead links** |
> | **G-4.1** update `docs/FOLDER_MAP.md` | **DONE 2026-10-03** — rewritten from the tree on disk: Phase-1 arrivals, sets A–U (noting M/N/O/Q/R do not exist), `ml-platform-context-pack/`, `mcp/`, `meridian-design-system/`, `images/`, `agent/`, `HARNESS_CONFORMANCE.md`, `LOCAL_TIER_FINDINGS.md`, ADRs 0001–0048, and the `Specs/` vs `specs/` case note; 0 dead links. Was: **OPEN** | it was touched and does show `docs/plans/plan-sets/`, but still reads "Last Updated: 2026-06-11", omits every Phase-1 arrival (`NEWCOMERS.md`, `procedures/AGENT.md`, `governance/COMPLACENCY_LOG.md`, both `architecture/latency-*.md`) — which is exactly the list G-4.1 required — and caps plan sets at "A–G" when L exists |
> | **G-4.2** `docs/README.md` as the nav hub | **Done** | seven-section index, one line per entry, zero broken relative links |
> | **G-4.3** root `README.md` points at `docs/` | **Done** | 87 lines (< 100 as specified), documentation table, zero broken links; `5febac2` cut 136 lines |
> | **G-4.4** update `add-plan.md` / `add-doc.md` | **Done differently** | both route to `docs/plans/plan-sets/`, not the planned `docs/plans/set-X/`. **Caveat:** `add-doc.md` names the spec index twice as `docs/specs/README.MD`, the case-variant — see remainder item 4 |
>
> **Disposition: closed as substantially complete (11 of 13), with a small
> honest remainder.** The restructuring goal — one navigable `docs/` tree —
> holds. What did not happen is the *index maintenance* the moves created work
> for, and three months of building has widened that gap.
>
> ### Open remainder — five items, all documentation hygiene, none blocking
>
> These are not worth a new plan set. They are a single cleanup pass. Item 1
> turned out to be already fixed in the coordinator's uncommitted tree, so the
> live remainder is **four**. Every claim below was re-tested against both this
> branch's HEAD *and* the shared working tree, because the two differ.
>
> | # | Item | Why |
> |---|---|---|
> | **1** | ~~Rewrite `docs/plans/README.md`~~ — **DONE** (the residual set-A header was fixed 2026-10-03: it now opens `# docs/plans/plan-sets/set-A/` and says there is one copy) | At this branch's HEAD it carries 11 dead links (every phase file it indexes moved to `docs/plans/plan-sets/set-A/`, the master plan to `docs/plans/reference/`) and lists nine Set A phases as "Pending". **In the coordinator's uncommitted working tree all 16 links resolve and a note explains the Pending rows are historical.** Roadmap gap **G-7 is genuinely fixed**; it is simply not committed yet. Nothing to do here beyond landing that commit. Residual: `docs/plans/plan-sets/set-A/README.md` still opens `# plans/` and claims two canonical copies of the plans exist, which is no longer true |
> | **2** | ~~Rebuild `docs/archive/README.md`~~ — **DONE 2026-10-03**, 0 dead links | **24 dead links** to the files G-3.1 deleted. This is literally G-3.1's own sub-step 4 |
> | **3** | ~~Finish `docs/FOLDER_MAP.md` (G-4.1)~~ — **DONE 2026-10-03** | never received the Phase-1 arrivals, and is now blind to `ml-platform-context-pack/`, `mcp/`, `meridian-design-system/`, `images/`, `HARNESS_CONFORMANCE.md`, `LOCAL_TIER_FINDINGS.md` and `artifact.md` |
> | **4** | Collapse the `docs/specs/README.MD` ⟷ `README.md` case-variant — **BLOCKED:HUMAN (2026-10-03).** Confirmed: on this checkout both index entries map to one physical file (`docs/Specs/README.md`), and git reports it as a modification of `README.MD`, the owner's standing no-touch file, which has uncommitted edits. Collapsing the pair (`git rm --cached` one name) and fixing its 10 dead links would both write that file, so neither was done | git tracks **both**; on a case-insensitive Windows checkout only one materialises, so edits to the wrong one vanish silently. They are byte-identical today only because `680b30c` hand-synced them. `docs/procedures/add-doc.md` tells contributors to update the **uppercase** one. This is the only case-fold collision in the whole repository |
> | **5** | Decide the fate of the repo-root `research/` directory — **BLOCKED:HUMAN; recommendation below** | a stale second research tree (`CONCLUSIONS.md` 67 KB, `OPEN_QUESTIONS.md` 35 KB, `SKILL.md`), last touched 2026-06-10, shadowing the live `docs/research/` copies. Set G's charter was one tree, but it enumerated only the six root `.md` files, so this slipped through. **Deleting 109 KB of content is a human yes/no, not an agent's call** |
>
> Also noted, outside Set G's scope: `docs/specs/README.md` carries **10 dead
> links** (including three into a `legacy/` tree that no longer exists), and
> `docs/adr/README.md` — which has **zero** dead links — simply omits ADRs
> **0012, 0013 and 0014**, an index gap rather than a broken link.
>
> **2026-10-03 follow-up.** ADRs 0012–0014 now have index rows in
> `docs/adr/README.md` (0012 and 0013 marked as Python-era historical, 0014
> marked as partially superseded by ADR-0044). The 10 dead links in
> `docs/specs/README.md` are **not fixed**, because that name is the same physical
> file as `README.MD` (item 4), which agents may not edit. The ten targets, for
> whoever edits it:
> `../CANONICAL_SPEC_INDEX.MD` (lines 3, 15) → `../archive/CANONICAL_SPEC_INDEX.MD`;
> `../CANONICAL_MODULE_MAP.MD` (17) → `../archive/CANONICAL_MODULE_MAP.MD`;
> `../FULL_AUDIT.md` (26) → `../governance/full_audit.md`;
> `BACKTESTING.MD` (41) → `FEAT-002-backtesting.md`;
> `../MIGRATION_TO_SPEC_PIPELINE.MD` (48) → `../architecture/migration_to_spec_pipeline.md`;
> `../SYSTEM_WALKTHROUGH.MD` (49) → `../architecture/system_walkthrough.md`;
> the three `../../legacy/decision_pipeline/docs/*.MD` links (38, 46, 50) have no
> surviving target and should become plain text marked "deleted with the Python tree".
>
> Still dead and deliberately left alone: about 400 relative links inside
> Python-era documents (`QUEUE_*`, `architecture/latency-*.md`,
> `special-plans/RESTART_DATA_HEALTH_AND_FORECASTER_PLAN.MD`, `operations/*`, the
> old `*.MD` specs, BRAINSTORM BS-001/002). They point into the deleted Python
> tree or the flat pre-2026-06-11 layout. Rewriting history-of-record documents
> was outside Set G's charter.
>
> ### Recommendation on the repo-root `research/` directory (item 5): a human decision
>
> Measured 2026-10-03: the last commit to touch it is `b79fd0ba` (2026-06-10). All
> three files differ from their `docs/research/` counterparts. `CONCLUSIONS.md` is
> 67 KB against the live 143 KB, which is the G-1.3 merge and broader in scope.
> `OPEN_QUESTIONS.md` is 35 KB against 32 KB with an **identical heading set**, so
> the root copy is wordier row for row rather than broader. `SKILL.md` differs in
> its working rules. Nothing in the repo links into the root tree.
>
> **Recommendation: delete `research/` in one commit**, after a one-time row-level
> diff of `OPEN_QUESTIONS.md` confirms that no OQ row text exists only in the root
> copy (if one does, carry it into `docs/research/OPEN_QUESTIONS.md` first). Git
> history keeps every byte, so the deletion is reversible. Rejected alternative:
> leaving a redirect stub `research/README.md`. It keeps a second tree alive that
> agents keep finding and citing. Not executed here, because deleting 109 KB of
> owner content is not an agent's call.

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
| 0 | [phase-0.md](phase-0.md) | Content Audit | 1 | **100%** | Done — `docs/archive/AUDIT.md`, 41 entries classified |
| 1 | [phase-1.md](phase-1.md) | Root File Migration | **5** (not 3) | **100%** | Done — all six root `.md` files moved; the `COMPLACENCY_LOG.md` cross-ref was fixed 2026-10-03 |
| 2 | [phase-2.md](phase-2.md) | Plans Migration | 2 | **100%** | G-2.1 done (at `docs/plans/plan-sets/`); G-2.2 DONE 2026-10-03 |
| 3 | [phase-3.md](phase-3.md) | Archive Prune | 1 | **100%** | 24 duplicates deleted in `5febac2` (the audit summary said 22; its table and the commit both show 24); `docs/archive/README.md` rebuilt 2026-10-03 |
| 4 | [phase-4.md](phase-4.md) | Index & Navigation | 4 | **100%** | G-4.2/4.3/4.4 done; G-4.1 DONE 2026-10-03 |

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
| 2026-06-13 | 0–4 | build | Commit `5febac2` executed essentially the whole set the day it was written. **The checkboxes were never ticked**, which is why this file read `0 / 11` for three months. |
| 2026-09-15 | all | audit | **Set closed at 11 / 13** by the plan-hygiene track (ROADMAP §9, human gate H-5). Real task count is 13, not 11 — `phase-1.md` holds 5 tasks, the phase table said 3. Five hygiene items re-cut into §"Open remainder" in the header; none blocks any other work, and one of the five (G-7 / `docs/plans/README.md`) proved to be already fixed in the coordinator's uncommitted tree, leaving four. |
| 2026-10-03 | 1–4 | hygiene | DOCS track: G-2.2, G-3.1 sub-step 4 and G-4.1 closed (13 / 13). ADR 0012–0014 index rows added; set-A README header fixed. Remainder items 4 (specs README case-variant and its 10 dead links) and 5 (repo-root `research/`) are BLOCKED:HUMAN, with the exact fixes and a recommendation recorded in the header. |
