# Phase 3 — Archive Prune

**Completion: 0% (0 / 1 tasks)**

**Prerequisite:** Phase 0 (G-0.1) must be complete. The audit table must exist
before any file is deleted.

**Goal:** Delete confirmed DUPLICATE files from `docs/archive/` based on the
Phase 0 audit. Absorb PARTIAL content into live files. Leave UNIQUE content
in place.

---

## Tasks

### ☐ G-3.1 Delete duplicate archive files per audit inventory — M

**Why.** `docs/archive/` currently contains 30+ files from past reorganisations.
Many are older versions of files now maintained in `docs/adr/`, `docs/specs/`,
`docs/governance/`, and `docs/reports/`. Keeping duplicates causes confusion
about which version is authoritative and bloats the docs tree.

**Prerequisite check:** Open the audit table from G-0.1 (either embedded in
`plans/set-G/phase-0.md` or at `docs/archive/AUDIT.md`). Proceed only if
every file has a classification.

**Steps:**

1. **Delete DUPLICATE files:**
   For each file classified DUPLICATE in the audit table:
   ```bash
   git rm docs/archive/<filename>
   ```
   Verify the live equivalent exists and is current before each deletion.

2. **Absorb PARTIAL files:**
   For each file classified PARTIAL:
   - Read the live equivalent and the archive file side by side.
   - Copy the unique sections from the archive file into the appropriate
     live document (do not alter canonical ADR or spec content — only
     append/update supplementary sections).
   - After absorption is verified, `git rm` the archive file.

3. **Leave UNIQUE files:**
   Files classified UNIQUE remain in `docs/archive/`. Do not delete them.
   Consider adding a brief note at the top: `<!-- Historical record: <date> -->`
   if there is risk of confusion with current content.

4. **Update archive README (if one exists):**
   If `docs/archive/` has a README, update it to reflect which files remain
   and why.

**Files:**
- All files under `docs/archive/` classified DUPLICATE or PARTIAL in the audit
- Any live files that absorb PARTIAL content

**Acceptance criteria:**
- Every file deleted is listed as DUPLICATE in the Phase 0 audit table.
- No UNIQUE-classified file is deleted.
- PARTIAL files are deleted only after their unique content is absorbed and
  the absorption is verifiable in the live file's git diff.
- `docs/archive/` is not empty after this task (UNIQUE files remain).
- The commit message for the deletion batch cites the audit table (e.g.,
  "docs(archive): prune 18 duplicate files per G-0.1 audit").
