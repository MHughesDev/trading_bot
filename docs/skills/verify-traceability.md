# Skill: Verify Traceability

## Purpose
Run a structural integrity check across the whole `docs/` tree so the cross-references that hold
the design together — IDs, coverage, derivation freshness, links — are proven intact rather than
assumed.

## When to Use
- Before handing a plan to execution (`skills/execute-plan.md`)
- At the end of a design session, after creating or editing specs, ADRs, plans, or the artifact
- Periodically on a long-running project to catch reference drift
- Any time a reference looks wrong and you want to know what else might be broken

---

## Procedures Used
- `procedures/verify-traceability.md`

---

## Workflow

### 1. Run the full procedure, not a spot check
The value is in completeness. A single broken `Q-N` reference usually means others exist —
run every check in `procedures/verify-traceability.md`, not just the one that prompted you.

### 2. Classify failures before acting
Group what you find:
- **Breaks traceability** — a reference that does not resolve. Fix before any downstream work.
- **Incomplete design** — an uncovered acceptance criterion or a stale plan. This often points at
  missing upstream work (a spec or task that was never written), not a typo. Surface it.
- **Hygiene** — index drift, a broken link with an obvious target. Safe to fix directly.

### 3. Fix hygiene, surface the rest
Apply hygiene fixes directly. For anything that breaks traceability or reveals an incomplete
design, surface it to the user with the likely cause — do not paper over a real gap by inventing
the missing artifact.

### 4. Re-run after fixing
If you fixed anything, run the checks again. Fixes can expose or create new mismatches.

---

## Tips

- This skill verifies structure, not correctness. It confirms `FEAT-001 §3.1.C` exists and is
  cited — not that the requirement is *good*. Content quality is the job of the create skills.
- The most common real failure is a missed re-derivation trigger: a spec changed but a plan that
  depends on it is still marked `Current`. Step 6 catches it.
- An undefined `SC-`/`FM-` citation almost always means the artifact was never fully filled in.
  That is a signal to return to `skills/create-artifact.md`, not to invent the missing condition.
- Run this before `execute-plan` so an agent never starts implementing against a broken plan.
