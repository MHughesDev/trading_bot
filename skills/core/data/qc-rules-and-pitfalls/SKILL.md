---
name: qc-rules-and-pitfalls
description: Use before trusting any bar series. Explains the data QC grade (A-D), what grade C and D mean for experiments, and the common ways a series misleads you.
---
# QC rules and pitfalls

Every bar response carries `manifest.qc_grade`. Read it before you read the rows.

## What the grade does

- `A`/`B`: usable.
- `C`: an Experiment may use it only with an approved waiver (`data_grade_c_needs_waiver`).
- `D`: refused outright (`data_grade_d_refused`). Under half the expected bars are present,
  or a quarter of the window is missing. No waiver covers it.
- `ungraded`: nobody has graded this series. It is not refused, and it is not graded. Say so
  in any report that uses it.

## Pitfalls that look like signal

- **Gaps are not zero returns.** A missing hour is an unknown, not a flat hour. Do not
  forward-fill across a gap and then measure volatility.
- **Flat bars.** A run of identical OHLC with zero volume is a collector outage or an illiquid
  venue, not a calm market.
- **The cutoff clips you silently.** `manifest.cutoff_applied` names the boundary when your
  window ran past the project's research cutoff. A short result is not a bug.
- **Revisions.** A bar can be restated. Pin `as_of` (and record `manifest.data_snapshot_id`)
  so a rerun reads the same data.
- **Backfilled knowledge time.** A row whose knowledge time was stamped by a backfill was not
  knowable at its own timestamp. Forward evidence excludes it.
- **Absent is not empty.** A response with `null` rows and `measured: false` means the series
  was never collected here. An empty list means it was collected and nothing happened.

## When in doubt

Run the `data_qc` job on the window before the experiment, not after it.
