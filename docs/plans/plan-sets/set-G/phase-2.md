# Phase 2 — Plans Migration

**Completion: 0% (0 / 2 tasks)**

**Goal:** Move the `plans/` folder from the repo root into `docs/plans/` so
that all documentation lives under a single `docs/` tree. Update every
internal cross-reference so no file in the repo points to the old `plans/`
location.

---

## Tasks

### ☐ G-2.1 Move plans/ → docs/plans/ (git mv) — M

**Why.** `plans/` at the repo root sits alongside `docs/` with no structural
connection between them. Moving it inside `docs/` creates a single authoritative
documentation tree and makes it discoverable from `docs/README.md`.

`plans/` contains:
- `README.md` (plans index)
- `set-C/` — 68 issues + 15 agent guides + MASTER.md
- `set-D/` — MASTER.md + 7 phase files
- `set-E/` — MASTER.md + future-scope.md + 6 phase files
- `set-F/` — MASTER.md + 5 phase files (new, this session)
- `set-G/` — MASTER.md + 5 phase files (this plan)

**Steps:**
```bash
git mv plans docs/plans
```

This single `git mv` moves the entire tree and preserves git history for all
files within it. Verify with:
```bash
git log --follow docs/plans/set-E/MASTER.md
```

**Note:** After this move, the **current file you are reading** will be at
`docs/plans/set-G/phase-2.md`. All subsequent phase and MASTER files in
set-G will be at the new path. Update internal self-references if any.

**Files:**
- `plans/` (entire directory tree) → `docs/plans/`

**Acceptance criteria:**
- `plans/` no longer exists at repo root.
- `docs/plans/` exists and contains all plan sets.
- `git log --follow docs/plans/set-E/MASTER.md` shows history from before the move.
- `git status` shows the moves as renames, not add+delete pairs.
- CI passes (no Makefile, script, or CI config references `plans/` directly —
  verify with `grep -r '"plans/' .github/ Makefile scripts/ 2>/dev/null`).

---

### ☐ G-2.2 Update all cross-references from plans/ to docs/plans/ — M

**Why.** After the move, any file that links to `plans/…` will have a broken
link. This includes procedure docs, the root `README.md`, `docs/README.md`,
plan MASTER files that reference each other, and potentially ADRs or specs.

**Systematic approach:**
1. Find all references:
   ```bash
   grep -r "\bplans/" --include="*.md" -l docs/ README.md
   ```
2. Update each to `docs/plans/` (or use a relative link where appropriate).
3. Key files expected to need updates:
   - `README.md` (root) — if it links to any plan set
   - `docs/README.md` — add `plans/` section to navigation
   - `docs/procedures/add-plan.md` — references `plans/` directory path
   - `docs/FOLDER_MAP.md` — lists directory structure
   - Plan MASTER files that cross-reference other plan sets (e.g.,
     `set-E/MASTER.md` notes "see `future-scope.md`" — relative links are
     unaffected, but any absolute paths from root must update)
   - `docs/procedures/execute-plan.md` — may reference `plans/` structure

4. Verify no broken links remain:
   ```bash
   grep -r "\bplans/" --include="*.md" . | grep -v "docs/plans/"
   ```
   (The only matches after this should be inside `docs/plans/` files themselves
   using relative links like `[phase-0](phase-0.md)` — those are fine.)

**Files:**
- `README.md`
- `docs/README.md`
- `docs/FOLDER_MAP.md`
- `docs/procedures/add-plan.md`
- `docs/procedures/execute-plan.md`
- Any other `.md` file found by the grep above

**Acceptance criteria:**
- `grep -r "\bplans/" --include="*.md" . | grep -v "docs/plans/"` returns no
  matches that are absolute-path links (relative links inside docs/plans/ are OK).
- `docs/procedures/add-plan.md` instructs contributors to create plan sets
  under `docs/plans/set-X/`.
- All links from `docs/README.md` to plan sets resolve correctly.
