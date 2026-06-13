# Phase 1 — Root File Migration

**Completion: 0% (0 / 5 tasks)**

**Goal:** Move the 6 documentation `.md` files at the repo root into
appropriate subdirectories under `docs/`. `README.md` stays at root.
Use `git mv` for every move to preserve git history.

---

## Tasks

### ☐ G-1.1 Move AGENT.md → docs/procedures/AGENT.md — S

**Why.** `AGENT.md` contains instructions and context for AI coding agents
operating in this repository. It is not a project overview (that's `README.md`)
— it belongs alongside the other procedural guides in `docs/procedures/`.

**Steps:**
```bash
git mv AGENT.md docs/procedures/AGENT.md
```

Update any cross-references to `AGENT.md`:
- Search: `grep -r "AGENT.md" --include="*.md" .`
- Update found links to `docs/procedures/AGENT.md`.

**Files:**
- `AGENT.md` → `docs/procedures/AGENT.md`
- Any `.md` files found to link to `AGENT.md`

**Acceptance criteria:**
- `AGENT.md` no longer exists at repo root.
- `docs/procedures/AGENT.md` exists with identical content.
- No `.md` file in the repo has a broken link to the old path.
- `git log --follow docs/procedures/AGENT.md` shows file history.

---

### ☐ G-1.2 Move NEWCOMERS.md → docs/NEWCOMERS.md — S

**Why.** `NEWCOMERS.md` is the onboarding guide for new contributors. It should
live under `docs/` as a top-level entry point, visible alongside `docs/README.md`
and `docs/glossary.md`.

**Steps:**
```bash
git mv NEWCOMERS.md docs/NEWCOMERS.md
```

Update cross-references:
- Search: `grep -r "NEWCOMERS.md" --include="*.md" .`
- Update root `README.md` if it links to `NEWCOMERS.md`.

**Files:**
- `NEWCOMERS.md` → `docs/NEWCOMERS.md`

**Acceptance criteria:**
- `NEWCOMERS.md` no longer exists at repo root.
- `docs/NEWCOMERS.md` exists with identical content.
- No broken links remain.

---

### ☐ G-1.3 Resolve and move CONCLUSIONS.md → docs/research/CONCLUSIONS.md — S

**Why.** `CONCLUSIONS.md` at root is a research summary. `docs/research/CONCLUSIONS.md`
already exists. Before moving, compare the two files to determine whether they
are the same, one is a subset of the other, or they cover different content.

**Merge strategy:**
1. `diff CONCLUSIONS.md docs/research/CONCLUSIONS.md`
2. If root version is a superset or has unique sections: merge unique sections
   into `docs/research/CONCLUSIONS.md`, then delete root version.
3. If root version is identical or older: delete root version directly.
4. Either way: root `CONCLUSIONS.md` must not exist after this task.

**Steps:**
```bash
# After merging (if needed):
git rm CONCLUSIONS.md
git add docs/research/CONCLUSIONS.md
```
If the root file has entirely new content:
```bash
# Replace the existing file, preserving history is less important here
# since both files will be combined
```

**Files:**
- `CONCLUSIONS.md` (root) → merged into or replaced by `docs/research/CONCLUSIONS.md`

**Acceptance criteria:**
- `CONCLUSIONS.md` no longer exists at repo root.
- `docs/research/CONCLUSIONS.md` contains all non-duplicate content from both files.
- Cross-references to `CONCLUSIONS.md` updated to `docs/research/CONCLUSIONS.md`.

---

### ☐ G-1.4 Move COMPLACENCY_LOG.md → docs/governance/COMPLACENCY_LOG.md — S

**Why.** `COMPLACENCY_LOG.md` is a tech-debt tracking document. `docs/governance/`
already contains audit and process docs (`full_audit.md`, `audit_code_review.md`).
It belongs there.

**Note:** Set E Phase 0 progress log references `COMPLACENCY_LOG.md` v2 as its
source. After moving, update any such internal references in the plan files.

**Steps:**
```bash
git mv COMPLACENCY_LOG.md docs/governance/COMPLACENCY_LOG.md
```

Update cross-references:
- Search: `grep -r "COMPLACENCY_LOG" --include="*.md" .`
- Key files to check: `plans/set-E/MASTER.md` (references it in the overview),
  `plans/set-E/phase-*.md` files.

**Files:**
- `COMPLACENCY_LOG.md` → `docs/governance/COMPLACENCY_LOG.md`
- `plans/set-E/MASTER.md` (update reference)

**Acceptance criteria:**
- `COMPLACENCY_LOG.md` no longer exists at repo root.
- All cross-references updated.
- `git log --follow docs/governance/COMPLACENCY_LOG.md` shows file history.

---

### ☐ G-1.5 Move LATENCY_ISSUES_*.md → docs/architecture/ — S

**Why.** Both latency analysis documents are architecture-level content.
`docs/architecture/` already contains `system_walkthrough.md`, `monitoring.md`,
and similar files.

Rename to kebab-case (consistent with other files in that directory):

**Steps:**
```bash
git mv LATENCY_ISSUES_COMPREHENSIVE.md docs/architecture/latency-analysis.md
git mv LATENCY_ISSUES_UNIFIED_TABLE.md docs/architecture/latency-table.md
```

Update cross-references:
- Search: `grep -r "LATENCY_ISSUES" --include="*.md" .`

**Files:**
- `LATENCY_ISSUES_COMPREHENSIVE.md` → `docs/architecture/latency-analysis.md`
- `LATENCY_ISSUES_UNIFIED_TABLE.md` → `docs/architecture/latency-table.md`

**Acceptance criteria:**
- Both source files no longer exist at repo root.
- Destination files exist with identical content.
- File names match kebab-case convention of the `docs/architecture/` directory.
- No broken cross-references remain.
