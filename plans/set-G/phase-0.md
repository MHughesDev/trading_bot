# Phase 0 — Content Audit

**Completion: 0% (0 / 1 tasks)**

**Goal:** Catalogue every file in `docs/archive/` before anything is deleted.
Phase 3 (archive prune) is blocked on this audit. The output is a checklist
that labels each file as "duplicate of live doc" (safe to delete) or "unique
historical content" (keep).

---

## Tasks

### ☐ G-0.1 Catalogue docs/archive/ — identify duplicates vs unique content — M

**Why.** `docs/archive/` contains 30+ files accumulated from past
reorganisations (legacy ADRs, older spec drafts, migration plans, audit
reports, microservices analysis). Before any deletions happen in Phase 3,
every file must be categorised. Deleting without this step risks losing unique
context that isn't captured anywhere in the live docs.

**Approach:**
1. List every file in `docs/archive/` (including subdirectories).
2. For each file, open it and check whether its substantive content is covered
   by a current file in `docs/adr/`, `docs/specs/`, `docs/architecture/`,
   `docs/governance/`, or `docs/reports/`.
3. Classify each as one of:
   - **DUPLICATE** — 100% superseded by a named live file; safe to delete.
   - **PARTIAL** — overlaps but has unique sections; live file should absorb
     the delta before deletion.
   - **UNIQUE** — content not present elsewhere; keep in archive or promote to
     a live subdirectory.
4. Produce an audit table and embed it in this task as a completion artifact
   (update the file or add `docs/archive/AUDIT.md`).

**Expected classifications (to verify during audit):**
- Legacy ADRs pre-dating the current `docs/adr/` set → likely DUPLICATE once
  numbered ADRs cover the same decisions.
- `FULL_AUDIT.md`, `CANONICAL_MODULE_MAP.MD`, `CANONICAL_SPEC_INDEX.MD` →
  likely PARTIAL (current `docs/reports/` covers audits; check for gaps).
- Microservices analysis, early migration plans → likely UNIQUE historical
  context.
- Anything referencing the old microservices architecture that was replaced →
  UNIQUE (historical) but not needed in live docs.

**Files to read (at minimum):**
- All files directly under `docs/archive/`
- All files in `docs/archive/archive/` (nested subdirectory)

**Output:** Embed a markdown table in this phase file (or `docs/archive/AUDIT.md`):

```markdown
| File | Classification | Live Equivalent (if DUPLICATE/PARTIAL) | Notes |
|------|---------------|----------------------------------------|-------|
| docs/archive/FULL_AUDIT.md | DUPLICATE | docs/reports/AUDIT_REPORT_2026-04-13_full.md | Same audit, newer version is live |
| ... | ... | ... | ... |
```

**Acceptance criteria:**
- Every file in `docs/archive/` (including subdirectories) appears in the table.
- Each file has a classification (DUPLICATE / PARTIAL / UNIQUE).
- DUPLICATE entries cite the specific live file they are superseded by.
- PARTIAL entries note which sections are unique (to be absorbed by the live
  file owner in Phase 3).
- The table is committed so Phase 3 can reference it directly.
