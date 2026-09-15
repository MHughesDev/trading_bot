# Phase 2 — Quality and eligibility

**Completion: 0% (0 / 6 tasks)**

**Requirement IDs:** DA-07.
**Spec:** [DATA-005 §7](../../../specs/DATA-005-data-api-v2.md) (the checks, the grades, the
refusal); `config/data_qc.toml`.
**Depends on:** Phase 1 (there must be history to grade), Phase 0 (the census supplies the
coverage check's inputs).
**Blocks:** Phase 4.
**Migration range:** 0100–0101.
**Parallel with:** Phase 3.

DA-07 landed in Set L Phase 6 as grade *gating*, but the grading itself is thin and the §7
check battery is not built. This phase makes a grade mean something, then makes it binding.

**Order matters here and it is deliberate: grades are published (P-2.6) before they are
enforced (P-2.3).** Switching on refusal without first knowing how many existing series would
be refused is how a correctness improvement becomes an outage.

---

### P-2.1 The check battery — M
`NOT STARTED`

The eight §7 checks as one deterministic job per `(instrument, timeframe, window)`:

| Check | Rule |
|---|---|
| Coverage | expected vs present, gap count, longest gap, % missing (reuses P-0.1) |
| Revisions | count of revised bars in the window |
| Bad ticks | `abs(log return) > 8·σ_rolling` reversing ≥ 80% on the next bar; price ≤ 0; `H < max(O,C)` or `L > min(O,C)` |
| Stale runs | ≥ N consecutive identical closes with zero volume, N per timeframe |
| Volume | zero-volume share; spikes > 20× rolling median |
| Timestamps | `available_time ≥ event_time`; monotonic; timezone consistency |
| Cross-source | median absolute deviation against a second venue where one exists |
| Survivorship | flag when the instrument or universe membership is post-selected |

Thresholds come from `config/data_qc.toml` with the spec's defaults; the job records which
config version produced a report, so a grade is reproducible.

**Files:** `crates/dataplane/src/qc/`, `config/data_qc.toml`.
**Acceptance:** each check fires on a planted fixture exercising exactly that failure and stays
silent on a clean series; the job is deterministic — same input, same config, byte-identical
report; the config version is recorded on every report.
**Traces:** DATA-005 §7; DA-07.

### P-2.2 Grades A–D and `data_qc_reports` — M
`NOT STARTED`

The grade function exactly as specified — A all pass; B gaps < 0.5% and no unrepaired bad
ticks; C material, gaps < 5% or repaired bad ticks; D gaps ≥ 5%, unrepaired bad ticks, or any
timestamp violation — persisted to `data_qc_reports` (migration 0100) with the reasons, not
just the letter. A grade with no reasons is unreviewable and unappealable.

The grade is cached into the instrument profile and attached to **every manifest** the Data API
emits.

**Files:** `crates/dataplane/src/qc/grade.rs`, migration 0100, manifest emission in
`crates/api/src/routes/data.rs`.
**Acceptance:** the four grades are produced from planted fixtures at each boundary, including
both sides of 0.5% and 5%; a timestamp violation forces D regardless of every other check
passing; every manifest carries the grade and the report id.
**Traces:** DATA-005 §7, §6 (manifests); DA-07.

### P-2.3 Refusal at Experiment creation, and the waiver — M
`NOT STARTED`

`POST /api/hypotheses` and `POST /api/backtest/experiments` refuse grade D outright and refuse
grade C without an approved waiver, returning `422 data_quality_insufficient` **listing the
reasons** — the error is teachable or it is noise. The waiver is an
`approval_requests.kind='qc_waiver'` row (migration 0101), a human decision, recorded with who
approved it and why, and attached to the Experiment's lineage forever.

A waiver is evidence about the Experiment, never a property of the data.

**Files:** `crates/api/src/routes/{hypotheses,experiments}.rs`, migration 0101.
**Acceptance:** creating an Experiment on a grade-D series fails with the reasons enumerated;
grade C fails without a waiver and succeeds with one; the waiver id is on the Experiment's
lineage and appears in its report; no token scope can bypass the check, proven by a test per
scope.
**Traces:** DATA-005 §7; DA-07; ADR-0025 (enforcement at platform services).

### P-2.4 Cross-source deviation — S
`NOT STARTED`

Where a second venue covers the same instrument, compare and report the median absolute
deviation, and flag windows where the two disagree beyond a threshold. This is the check that
catches a venue quietly serving bad history during a backfill — the failure Phase 1 is most
likely to introduce and least likely to notice.

**Files:** `crates/dataplane/src/qc/cross_source.rs`.
**Acceptance:** two venues' histories of the same instrument produce a deviation series; a
planted divergence in one venue's window is flagged with its bounds; an instrument with only
one source reports `not_applicable` rather than passing vacuously.
**Traces:** DATA-005 §7 (cross-source).

### P-2.5 Survivorship — S
`NOT STARTED`

Flag when an instrument or universe membership is post-selected — that is, when the set being
studied was chosen with knowledge of which members survived. The check reads catalog metadata
and the universe membership history from Phase 4, and reports the flag on the qc report.

Until Phase 4 lands, the check runs on instrument catalog metadata alone and reports
`insufficient_metadata` rather than passing — a vacuous pass here is worse than no check,
because survivorship is invisible in the numbers by construction.

**Files:** `crates/dataplane/src/qc/survivorship.rs`.
**Acceptance:** a universe defined from currently-listed members only is flagged; a universe
with a full membership history including delisted members is not; missing metadata reports
`insufficient_metadata` and never `pass`.
**Traces:** DATA-005 §7 (survivorship), §8; DA-08.

### P-2.6 Publish grades before enforcing them — M
`NOT STARTED`

Run the battery over every `(instrument, timeframe)` the platform holds, publish the grades on
the coverage page (P-0.2) beside the depth, and record the blast radius: how many series are C
or D, and which existing Experiments would have been refused.

**This task is sequenced before P-2.3's enforcement is switched on.** Its output is what makes
that switch a decision with a known cost rather than a surprise.

**Files:** none new — a run plus a written result in this phase file and a column on the
coverage page.
**Acceptance:** every held series has a published grade with reasons; the count of C and D
series is recorded here; enforcement (P-2.3) is not enabled in production until this task's
result is written down.
**Traces:** DATA-005 §7; Set P §8 risks.

---

## Phase exit

- Every series the platform holds has a reproducible grade with reasons, published.
- Grade D cannot enter an Experiment; grade C needs a recorded human waiver that stays on the
  lineage.
- The cost of enforcement was measured before enforcement was switched on.
