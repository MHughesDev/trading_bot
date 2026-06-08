# Procedure: Verify Traceability

## Purpose
Mechanically check that the web of cross-references holding the design together is intact —
that every ID referenced exists, every requirement is covered, every plan is current, and no
markdown link is broken. This is the guardrail that keeps the traceability from quietly rotting.

## Trigger
Use this procedure:
- At the end of a design session, before handing a plan to execution
- After any batch of changes to specs, ADRs, plans, or the artifact
- Periodically on a long-running project, to catch drift
- Whenever a reference looks suspicious (an `SC-`, `Q-`, or `§` that may not exist)

## Inputs
- Read access to the whole `docs/` tree
- No project-specific knowledge required — this procedure checks structure, not content

---

## Steps

Run each check below. For every failure, record it in the report (Step 8) — do not fix silently;
some failures indicate a real upstream gap that the user should see.

### 1. Open-question references resolve
For every `Q-N` referenced anywhere under `docs/` (specs, ADRs, plans, research, architecture),
confirm a matching row exists in `docs/open-questions.md`.
- **Fail:** a `Q-N` is cited but not registered → either register it or correct the reference.

### 2. Artifact ID references resolve
For every `SC-N` (success condition) and `FM-N` (failure mode) cited in any plan or spec,
confirm the ID exists in `docs/artifact.md`.
- **Fail:** an `SC-`/`FM-` is cited but not defined in the artifact → the artifact is incomplete
  or the citation is wrong. This is the exact class of gap that motivates this procedure.

### 3. Spec section references resolve
For every `<SPEC-ID> §N.M.X` reference (in plans, ADRs, other specs), confirm:
- the spec file `<SPEC-ID>-*.md` exists, and
- the cited section number exists within it.
- **Fail:** a `§` points at a missing spec or a non-existent section → fix the citation.

### 4. ADR references resolve
For every `ADR-NNNN` referenced, confirm `adr/NNNN-*.md` exists and its status is not
`Superseded` without the citing artifact acknowledging the supersession.
- **Fail:** a citation points at a missing or silently-superseded ADR.

### 5. Acceptance-criteria coverage (two-way)
For each formal plan whose `Status` is `Active` or `Complete`:
- **Forward:** every acceptance criterion (`§6.1.x`) of every spec in the plan's **Derived From**
  section is cited by at least one task, and every artifact `SC-N` maps to a milestone.
- **Backward:** every task has a source trace (`→ SPEC-ID §N.M.X` or `→ architecture §N`).
- **Fail:** an uncovered criterion (missing task) or an unsourced task (missing trace).

### 6. Derivation freshness
For each formal plan, read its `Derivation Status`.
- If `STALE`, report it — it must be re-derived before execution (see `add-plan.md`).
- Cross-check: for every spec/ADR modified more recently than a plan that lists it in **Derived
  From**, that plan should be `STALE`. If it is still `Current`, the re-derivation trigger was
  missed → mark it `STALE` and report.

### 7. Markdown links resolve
For every relative markdown link under `docs/`, confirm the target file exists.
- **Fail:** a broken relative link → fix the path.

### 8. Index completeness
For each folder with a `README.md` index (`adr/`, `specs/`, `plans/`, `research/`), confirm every
file in the folder has an index row and every index row points to a file that exists.
- **Fail:** an unlisted file or a dangling index row.

### 9. Verification evidence on implemented specs
For each spec whose `Status` is `Implemented`, confirm every acceptance criterion (§6.1.x) has a
real `Verified by:` value (not `[—]`) and a ticked checkbox.
- **Fail:** a spec marked `Implemented` with an unverified criterion → either record the evidence
  or the spec is not actually implemented; correct the status.

### 10. Write the report
Summarize results as a checklist, one line per check, `PASS` or `FAIL: <detail>`. Present failures
to the user grouped by severity:
- **Breaks traceability** (Steps 1–4, 7): a reference does not resolve — fix before proceeding.
- **Incomplete design** (Steps 5–6, 9): a gap, a stale plan, or an unverified "Implemented" spec — may need upstream work.
- **Hygiene** (Step 8): index drift — safe to fix directly.

Fix hygiene failures directly. For the others, surface the failure and the likely cause; only fix
when the correct resolution is unambiguous (e.g. a clear typo in an ID).

---

## Outputs
- A traceability report (PASS/FAIL per check), surfaced to the user
- Hygiene fixes applied directly (index drift, broken links with an obvious target)
- A list of substantive gaps (uncovered criteria, stale plans, undefined IDs) for the user to resolve

## Checklist
- [ ] All `Q-N`, `SC-N`, `FM-N`, `ADR-NNNN`, and `§N.M.X` references resolve
- [ ] Every active/complete plan passes two-way coverage
- [ ] No plan is `STALE` without being reported; no missed re-derivation triggers
- [ ] No broken relative markdown links
- [ ] Every folder index matches its folder contents
- [ ] Every `Implemented` spec has verification evidence on all its acceptance criteria
- [ ] Report surfaced to the user; substantive gaps flagged, not silently patched

## Related
- Skill: `skills/verify-traceability.md`
- Procedure: `procedures/add-plan.md` (coverage check and re-derivation it verifies)
- Procedure: `procedures/add-spec.md` (spec IDs and section addressing it checks)
