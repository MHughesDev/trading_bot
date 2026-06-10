---
Type: Formal
Status: Pending
Derived From: SYS-001, DATA-001, DATA-002, DATA-003, DATA-004, FEAT-001, COMP-001, COMP-002, COMP-003, COMP-004, INTG-001, ADR-0001, ADR-0002, ADR-0003, ADR-0004, ADR-0005, ADR-0006, ADR-0007, ADR-0008, ADR-0009, ADR-0010, ADR-0011, SC-1, SC-2, SC-3, SC-4, SC-5, SC-6, SC-7
Note: Canonical executable plans live in docs/plans/. This copy is the traceable documentation record. On any conflict, [deleted - see Phase 7]/ wins.
---

# Phase 7 — Cutover (parity, decommission Python, **finalize docs**, release)

> **Self-contained execution doc.** You need only: this file, [`../architecture.md`](../architecture.md),
> [`./rust-rewrite-master-plan.md`](./rust-rewrite-master-plan.md) §6 (definition of done), the
> live `docs/` workspace (stood up in Phase A), and `[deleted - see Phase 7]/` — which is still
> present at root as the permanent reference anchor and is the **last thing deleted**, as the final
> act of this phase.
>
> **This phase includes the FINALIZE documentation pass.** Phase A initialized the `docs/` workspace
> with the design-time intent (specs in `Draft`, acceptance criteria unfilled). Phase 7 closes every
> loop: specs advance to `Implemented`, evidence is recorded, operational procedures are written from
> the actual running system, and the full traceability verify runs. See P7-T05.

## Phase goal

After this phase, the Rust platform fully replaces the Python system: behavior parity has been
verified against `legacy_python/`, the entire `legacy_python/` tree is deleted, the `docs/` workspace
is finalized to the as-built state (every spec `Implemented`, every acceptance criterion evidenced,
operational procedures written from reality), the build is reproducible and release-tagged, and the
repository matches `docs/architecture.md` exactly with no orphan files.

## Prerequisites

- Phases B, 0, 1, 2, 3, 4, 5, 6 all complete with their exit criteria green.
- The React frontend runs entirely against the Rust backend.

## Invariants this phase must respect

- **Do not delete Python until parity is verified and recorded.** Deletion (P7-T03) happens only
  after the parity matrix (P7-T01) is green and reviewed.
- **No regression of money-safety guarantees.** All adversarial tests from Phases 0–6 must still be
  green at cutover.

---

## Tasks

### P7-T01 — Behavior parity matrix
- **Goal:** Verify the Rust system reproduces the Python system's observable behavior on the paths
  that matter.
- **Files:** `docs/parity-matrix.md` (the checklist + evidence).
- **Context:** Enumerate the behaviors to compare against `legacy_python/`: ingestion/normalization
  edge cases, bar building (incl. late-data revisions), risk-gate limit decisions, paper-fill +
  reconciliation behavior, strategy decisions on a fixed historical window, and backtest metrics on a
  fixed archive. For each, run the Rust path and (where feasible) the Python path on the same input
  and record agreement or an explained, intentional difference. Where the Python behavior was buggy,
  document that the Rust behavior is intentionally corrected.
- **Acceptance:** `docs/parity-matrix.md` covers every row above with evidence; differences are
  explained and signed off.
- **Depends on:** Phases 1–6.

### P7-T02 — Full adversarial test sweep
- **Goal:** Confirm every "decided mechanism" test from the spec is green (per
  [`../../research/OPEN_QUESTIONS.md`](../../research/OPEN_QUESTIONS.md) standing reminder).
- **Files:** none new — run the suite; record results in `docs/parity-matrix.md`.
- **Context:** Re-run: quarantine→replay, bar revision on late data, idempotent fills, ack-timeout
  query, no-lookahead replay, live/replay equivalence, reconciliation halt, freshness-respects-hours,
  kill-switch trips, tighten-only overrides, redelivered-intent idempotency. All must fire.
- **Acceptance:** the full `just test` (workspace + `tests/`) is green, including every adversarial
  test; results recorded.
- **Depends on:** P7-T01.

### P7-T03 — Decommission `legacy_python/`
- **Goal:** Remove the old Python system and any references to it.
- **Files:** delete `legacy_python/`; remove any remaining root Python artifacts (`.python-version`,
  `requirements.txt`, `pyproject.toml`, `.ruff_cache`, `.pytest_cache`, `trading_bot.egg-info`,
  Python `run/setup/doctor` scripts) that are no longer used; update `.gitignore`.
- **Context:** Only after P7-T01 and P7-T02 are green and reviewed. Grep the repo to ensure nothing
  (CI, docs, scripts, `justfile`) references `legacy_python/` or the old Python entrypoints before
  deleting.
- **Acceptance:** `legacy_python/` is gone; no file references it; `just test` and CI remain green;
  the repo tree matches [`../architecture.md`](../architecture.md) (no orphans, nothing missing).
- **Depends on:** P7-T02.

### P7-T04 — Structure conformance check
- **Goal:** Verify the repository matches the end-state file structure exactly.
- **Files:** optionally a small `xtask` subcommand (`cargo xtask check-structure`) that diffs the tree
  against the enumerated structure; otherwise a manual checklist.
- **Context:** Walk [`../architecture.md`](../architecture.md): every enumerated crate/app/file
  exists and every existing file is accounted for. Flag orphans and gaps.
- **Acceptance:** zero orphans, zero missing enumerated files (or documented, justified deviations
  with `architecture.md` updated to match reality).
- **Depends on:** P7-T03.

### P7-T05 — FINALIZE documentation workspace (the Phase A counterpart)
- **Goal:** Close every loop the Phase A initialize pass opened: advance specs to `Implemented`,
  record evidence on every acceptance criterion, write operational procedures from the actual running
  system, and reconcile the architecture map to the as-built reality. This is the documentation
  finalize pass.
- **Files touched in `docs/`:**
  - **Every spec** in `docs/specs/` — advance `Status` from `Draft` → `Implemented` (or `Approved`
    if implemented-but-not-yet-verified); tick each `§6.1.x` checkbox and fill in `Verified by:`
    with the evidence (test name, manual-test result + date, or link to the CI run from P7-T02).
  - **`docs/architecture.md`** — reconcile every component row and the repo-structure section to the
    as-built system; remove any remaining `[not yet decided]` markers; update the data-flow diagram
    if the shape shifted from design intent.
  - **`docs/research/OPEN_QUESTIONS.md`** — close any remaining `Open` questions resolved during the build
    (OQ-067 capital/liability, OQ-068 retention policy); add the resolving ADR or conclusion C-NNN.
  - **Operational procedures** (new, written from reality now that the system actually runs):
    `docs/procedures/operate-the-stack.md` (start/stop, kill switch, recovery, reconciliation alarms),
    `docs/procedures/add-a-venue.md` (the proven "collector + payload + metadata rows" checklist,
    validated by Phase 6 doing exactly this).
  - **API specs** — update the REST/WS `SYS`/`COMP` specs to match the shipped contract exactly.
  - **Root `README.md`** — quickstart reflects the Rust-only repo with the `docs/` workspace.
- **Context:** Phase A scaffolded the workspace with design intent; this task makes it reflect
  actuality. Follow [`../procedures/add-spec.md`](../procedures/add-spec.md) (the spec lifecycle
  steps) and [`../procedures/verify-traceability.md`](../procedures/verify-traceability.md) (the
  closing check). Operational procedures follow the authoring template in
  [`../procedures/README.md`](../procedures/README.md).
- **Acceptance:** every spec in `docs/specs/` is `Implemented` with all `Verified by:` fields
  filled and all checkboxes ticked; `architecture.md` matches the real repo with no `[not yet
  decided]` remaining; all open questions are either resolved or documented with a concrete
  resolution plan; the two operational procedures exist and match what Phase 6 actually proved;
  `verify-traceability` runs all 10 steps (including step 9, "Verification evidence on implemented
  specs") and reports PASS.
- **Depends on:** P7-T04 (repo tree is confirmed clean before we finalize the docs that map it).

### P7-T06 — Release
- **Goal:** A reproducible, tagged release of the Rust platform.
- **Files:** `.github/workflows/release.yml` (finalize), `Dockerfile` (multi-stage build of
  `platform` + collectors), version bump in the workspace.
- **Context:** The release workflow builds the binaries + the frontend, produces artifacts/images, and
  tags the version. Confirm `docker compose up` + the released image run the full stack.
- **Acceptance:** a tagged release builds reproducibly in CI and produces runnable artifacts.
- **Depends on:** P7-T05.

### P7-T07 — Delete `[deleted - see Phase 7]/` (the final act)
- **Goal:** Remove the refactor reference folder. This is the closing signal that the refactor is
  completely done — nothing else follows it.
- **Files:** delete `[deleted - see Phase 7]/` entirely from the repo root.
- **Context:** `[deleted - see Phase 7]/` has lived at the repo root, read-only and unmodified,
  throughout the entire refactor (Phases A–6) as the permanent reference anchor for the spec,
  architecture, file structure, and plan documents. Every piece of content it held has long since
  been migrated into `docs/` (Phase A) and is now fully implemented and evidenced (Phase 7). It no
  longer serves a purpose. Grep the repo to confirm nothing references it, then delete it. This is
  the **very last thing** done — after parity, after decommissioning Python, after finalizing docs,
  after the release is tagged.
- **Acceptance:** `[deleted - see Phase 7]/` is gone from the repo root; no file in the codebase,
  `docs/`, CI, scripts, or `justfile` references it; `just test` and CI remain green; this task
  being done means **the refactor is complete**.
- **Depends on:** P7-T06 (release tagged and verified).

---

## Phase exit criteria (and whole-refactor done)

- [ ] Parity matrix green; all intentional differences documented.
- [ ] Full adversarial test sweep green (`just test` including every adversarial test).
- [ ] `legacy_python/` deleted; no file references it; CI green.
- [ ] Repo tree matches `docs/architecture.md` (no orphans/gaps).
- [ ] **Documentation finalize pass complete (P7-T05):**
  - every spec `Implemented` with all `Verified by:` fields filled and boxes ticked;
  - `architecture.md` matches reality, no `[not yet decided]` remaining;
  - all open questions resolved or concretely deferred;
  - `docs/procedures/operate-the-stack.md` and `docs/procedures/add-a-venue.md` exist and match reality;
  - `verify-traceability` all 10 steps pass (including step 9 — evidence on implemented specs).
- [ ] Tagged, reproducible release builds and runs the full stack.
- [ ] [`./rust-rewrite-master-plan.md`](./rust-rewrite-master-plan.md) §6 definition of done is satisfied.
- [ ] **`[deleted - see Phase 7]/` deleted from the repo root (P7-T07). This is the last checkbox.
      When it is ticked, the refactor is done.**
