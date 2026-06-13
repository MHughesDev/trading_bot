# Phase 4 — Index & Navigation Updates

**Completion: 0% (0 / 4 tasks)**

**Goal:** After all file moves and prunes are complete, update the navigation
layer so readers can find everything from the top-level entry points: root
`README.md`, `docs/README.md`, and `docs/FOLDER_MAP.md`. Update procedure
guides so contributors know where to create new plans and docs.

---

## Tasks

### ☐ G-4.1 Update docs/FOLDER_MAP.md with new structure — S

**Why.** `docs/FOLDER_MAP.md` is the directory map of the `docs/` tree. After
Phases 1–3, the structure has changed:
- `docs/plans/` is new (moved from root `plans/`)
- `docs/procedures/AGENT.md` is new
- `docs/NEWCOMERS.md` is new
- `docs/research/CONCLUSIONS.md` is updated (merged)
- `docs/governance/COMPLACENCY_LOG.md` is new
- `docs/architecture/latency-analysis.md` and `latency-table.md` are new
- `docs/archive/` is smaller (after Phase 3)

**Steps:**
- Read the current `docs/FOLDER_MAP.md`.
- Add entries for every new file and directory.
- Remove entries for deleted archive files.
- Ensure `docs/plans/` tree is represented (at least the set-level directories;
  individual issue/phase files don't need to be listed).

**Files:**
- `docs/FOLDER_MAP.md`

**Acceptance criteria:**
- Every file added in Phases 1–3 appears in FOLDER_MAP.md.
- No deleted file remains listed.
- `docs/plans/` directory tree is represented.

---

### ☐ G-4.2 Update docs/README.md as the central navigation hub — S

**Why.** `docs/README.md` is the front page of the documentation. It should
give a reader a complete map of what is in `docs/` and where to go for:
architecture decisions (ADRs), canonical specs, plans, procedures, research,
governance, and onboarding.

**Content to add or update:**
- Add a **Plans** section linking to `docs/plans/README.md` and each plan set's
  MASTER.md.
- Add an **Onboarding** section linking to `docs/NEWCOMERS.md`.
- Add **Governance** entry for `docs/governance/COMPLACENCY_LOG.md`.
- Add **Architecture** entries for `latency-analysis.md` and `latency-table.md`.
- Add **Procedures** entry for `AGENT.md` (AI agent operating guide).
- Ensure the ADR and specs sections are present (they may already be).

**Files:**
- `docs/README.md`

**Acceptance criteria:**
- `docs/README.md` links to every top-level subdirectory with a one-line
  description of its contents.
- `docs/plans/` is linked and described.
- `docs/NEWCOMERS.md` is linked under an onboarding heading.

---

### ☐ G-4.3 Update root README.md to point to docs/ — S

**Why.** The root `README.md` is the GitHub landing page. It should direct
readers to `docs/README.md` for full documentation, and to `docs/plans/` for
the implementation roadmap. Currently it likely has inline content that
duplicates or conflicts with the now-consolidated docs.

**Approach:**
- Keep `README.md` short: project name, one-paragraph description, quick-start
  commands, and a "Documentation" section linking to `docs/README.md`.
- Do not delete substantive content; move any doc-level content from `README.md`
  into the appropriate `docs/` subdirectory (e.g. architecture overview →
  `docs/architecture/system_walkthrough.md` which already exists).
- Add explicit links:
  - `[Full documentation](docs/README.md)`
  - `[Architecture decisions](docs/adr/README.md)`
  - `[Implementation plans](docs/plans/README.md)`
  - `[Contributor guide](docs/NEWCOMERS.md)`

**Files:**
- `README.md`

**Acceptance criteria:**
- Root `README.md` links to `docs/README.md`.
- Root `README.md` links to `docs/plans/README.md`.
- Root `README.md` is under 100 lines (brief project overview + links only).

---

### ☐ G-4.4 Update docs/procedures/add-plan.md and add-doc.md — S

**Why.** Procedure guides tell contributors how to add new content. After the
move, `add-plan.md` must reference `docs/plans/set-X/` not `plans/set-X/`.
`add-doc.md` should reference the updated directory map.

**`docs/procedures/add-plan.md` changes:**
- Update any path references from `plans/set-X/` → `docs/plans/set-X/`.
- Update step "create the directory" to `mkdir docs/plans/set-X`.
- Update MASTER.md creation step to reference the new path.

**`docs/procedures/add-doc.md` changes:**
- Verify it references `docs/FOLDER_MAP.md` for the canonical directory list.
- Add a note that new plans go under `docs/plans/`.

**Files:**
- `docs/procedures/add-plan.md`
- `docs/procedures/add-doc.md`

**Acceptance criteria:**
- `add-plan.md` instructs contributors to create `docs/plans/set-X/` (not `plans/set-X/`).
- Following `add-plan.md` step-by-step produces a correctly placed plan set.
- `add-doc.md` references the updated folder structure.
