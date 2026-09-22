# Phase 2 — Quality and eligibility

**Completion: 83% (4 / 6 tasks). P-2.3 `NOT STARTED` — enforcement, deliberately last; see P-2.6.** P-2.2 was downgraded from `DONE` on 2026-09-21: `data_qc_reports` has no reader and no writer anywhere outside its own migration.

**Requirement IDs:** DA-07.
**Spec:** [DATA-005 §7](../../../specs/DATA-005-data-api-v2.md) (the checks, the grades, the
refusal); `config/data_qc.toml`.
**Depends on:** Phase 1 (there must be history to grade), Phase 0 (the census supplies the
coverage check's inputs).
**Blocks:** Phase 4.
**Migration range:** 0101 (0100 shipped 2026-09-15 as `data_qc_reports`).
**Parallel with:** Phase 3.

DA-07 landed in Set L Phase 6 as grade *gating*, but the grading itself is thin and the §7
check battery is not built. This phase makes a grade mean something, then makes it binding.

**Order matters here and it is deliberate: grades are published (P-2.6) before they are
enforced (P-2.3).** Switching on refusal without first knowing how many existing series would
be refused is how a correctness improvement becomes an outage.

---

### P-2.1 The check battery — M
`DONE`

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
`PARTIAL` — **downgraded from `DONE` on 2026-09-21.** The acceptance is
*"persisted to `data_qc_reports` … every manifest carries the grade and the
report id"*. The string `data_qc_reports` appears **nowhere in `crates/`,
`frontend/src/` or `xtask/`** — only inside its own migration. The table is live
with **0 rows, no reader and no writer**, and `routes/data.rs`'s manifest block
emits neither the grade nor the report id.

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
`DONE`

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
`DONE`

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
`DONE`

**Run 2026-09-15, config version `88f6845c91664a3e`, over every series in
`trading.market_bars_v2`.** Reproduce with
`cargo run -p dataplane --example qc_census`.

| Instrument | Venue | TF | Bars | Grade |
|---|---|---|---:|---|
| BTC-USD | coinbase | 1m | 93,453 | **D** |
| ETH-USD | coinbase | 1m | 44,387 | **D** |
| BTC-USD | kraken | 1m | 12,930 | **D** |
| SYN-PLANTED-AR1-424242 | synthetic | 1h | 5,000 | B |
| BTC-USDT | coinbase | 1h | 2,281 | C |
| BTC-USD | binance | 1m | 121 | B |
| ETH-USD | coinbase | 1h | 48 | B |
| BTC-USD | coinbase | 1h | 22 | **D** |
| ETH-USD | kraken | 1m | 17 | **D** |

**Blast radius: A 0 · B 3 · C 1 · D 5.** Switching on P-2.3 today would refuse
**five of nine series outright** and send a sixth for a human waiver. **No series
on the platform earns an A.**

### The defect this found on its first real run

**`BTC-USD/coinbase/1m` contains 2026-08-13 twice.** 2,879 rows for 1,440
distinct minutes — a whole day ingested a second time, plus one stray duplicate
on 08-14. Nothing had noticed: the duplicates are not gaps, not bad prices, and
not stale, so every other check in the battery passes over them. Only the
timestamp check looks for a minute that appears twice.

That is worth more than the grades. A duplicated day silently doubles the weight
of everything that happened in it, and a strategy tested across that window was
evaluated against a market that traded 2026-08-13 twice.

### Why no series reaches an A

Two reasons, and they are different in kind:

- **Cross-source and survivorship cannot run for most series.** One venue means
  no second opinion; no membership history means survivorship is unanswerable.
  Under the rule that an A requires all eight checks to have *passed* — and a
  check that could not run did not pass — those series cap at B. That is the
  design working, not a bug to tune away.
- **Three of the five Ds are coverage**, and those are the evidence clock
  showing up in the grades: 90%, 94% and 99.99% of the expected bars are absent
  because collection started recently and ran intermittently.

**And these coverage figures are optimistic.** The run took each series' expected
bar count from its own first and last timestamp, so it measures *internal* gaps
and not absence at the ends. The census (P-0.1) supplies the real expected count
and the production job must use it; this run did not, and says so rather than
letting the number read as authoritative.

### The decision this unblocks

**P-2.3 is not built**, and after this run that is the right order rather than an
omission. Enforcement today would refuse five of nine series, including every 1m
series with real depth — which is every series any current research would use.

The phase's mandated sequence is now satisfied in the only way that means
anything: the cost of enforcement is measured and written down *before* the
enforcing code exists, so whoever writes it is making a decision rather than
discovering a surprise.

**Recommended order before enabling:** de-duplicate 2026-08-13 (a data repair,
not a grading change), then re-run this census, then enable. The three
coverage-D series will stay D until Set P Phase 1's backfill lands, and that is
the correct answer rather than a threshold to loosen.

**Files:** `crates/dataplane/examples/qc_census.rs` (the reusable run).
**Acceptance:** every held series has a published grade with reasons — **met**,
nine of nine above; the count of C and D series is recorded here — **met**, 1 and
5; enforcement is not enabled in production until this result is written down —
**met**, and more strongly than asked: P-2.3 is unwritten.
**Traces:** DATA-005 §7; Set P §8 risks.

---

## Phase exit

- Every series the platform holds has a reproducible grade with reasons, published.
- Grade D cannot enter an Experiment; grade C needs a recorded human waiver that stays on the
  lineage.
- The cost of enforcement was measured before enforcement was switched on.
