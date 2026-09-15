# Phase 0 — Reconciliation and the first observation

**Completion: 0% (0 / 4 tasks)**

**Requirement IDs:** FE-REC-01…FE-REC-04, FE-G15.
**Spec:** ML platform SPEC §12.3 (Gate 15); HANDOFF §5.3; `artifact.md` SC-6, FM-4;
[COMP-002](../../../specs/COMP-002-execution-and-risk-gate.md).
**Depends on:** nothing. `crates/reconciliation` exists and is unwired.
**Blocks:** Phase 1, and Gate 15 everywhere.
**Migration range:** 0105–0106.
**Parallel with:** all of Set S and Set P.

Four tasks that turn the platform's most expensive unanswered question — *does the simulator
tell the truth?* — from unanswerable into merely unanswered-yet.

---

### T-0.1 Wire `crates/reconciliation` to the paper fill stream — M
`NOT STARTED`

The crate exists and nothing calls it. Subscribe it to paper fills and positions, and have it
maintain, per deployed strategy, the realised return series in the same shape the ledger stores
backtest return series in — same bar alignment, same numeraire handling (`Numeraire`, so USDT
is not silently summed as USD), same cost treatment.

Same shape is the whole task. Two series that are *nearly* comparable produce a divergence
number that means nothing.

**Files:** `crates/reconciliation/src/lib.rs`, `crates/execution` fill subscription,
migration 0105 (`forward_observation` table: deployment_id, bar, realised_return,
expected_return, source, observed_at).
**Acceptance:** a paper fill produces a `forward_observation` row within one bar; the realised
series and the backtest series for the same strategy are element-wise comparable without any
per-call adjustment; numeraire conversion is applied and a test asserts a USDT-denominated
strategy is not summed as USD.
**Traces:** SPEC §12.3; HANDOFF §5.3; ADR-P6 (`Numeraire`).

### T-0.2 Gate 15 reads the observations — M
`NOT STARTED`

`record_statistic` the forward-vs-expected comparison into the ledger so Gate 15 stops
returning `NotComputable`. The gate states its **observation window** and its **count** on the
verdict: "n observations over d days" is part of the verdict, not a footnote, because a gate
passed on four observations and one passed on four hundred are different claims.

**Only observed-knowledge rows count** (D-T1). A backfilled row is not forward evidence no
matter how recent.

**Files:** `crates/backtest/src/gates/`, `crates/ledger` statistic recording, migration 0106.
**Acceptance:** Gate 15 renders a verdict for a strategy with observations and `NotComputable`
with a stated reason for one without; the verdict carries the observation count and window; a
test plants a backfilled row in the window and asserts it does not enter the count.
**Traces:** SPEC §12.3 Gate 15; D-T1; ADR-P2-24.

### T-0.3 Divergence halts before the next order — M
`NOT STARTED`

SC-6 and FM-4: the system halts on position or balance divergence **before** submitting new
orders. Per-strategy thresholds declared at deployment (Phase 1) and recorded; when realised
diverges from expected beyond threshold, or when the internal position view disagrees with the
broker's, submission stops for that strategy and an alert fires.

The halt is the deliverable. A report that says "we diverged" after the next order went out is
the failure mode this exists to prevent.

**Files:** `crates/reconciliation/src/divergence.rs`, `crates/risk` pre-submit check.
**Acceptance:** a planted position divergence halts submission for that strategy before the next
order, proven by a test that counts orders reaching the venue adapter; the halt is visible in
the UI with its reason; clearing it requires an explicit human action that is recorded; **no
flag exists that continues on divergence**, asserted by a static check.
**Traces:** `artifact.md` SC-6, FM-4; COMP-002; D-T6.

### T-0.4 Deploy one hand-authored strategy and start the clock — M
`NOT STARTED`

The ops task, and the reason this phase is in Stage 0. Take a simple, hand-authored,
already-validated strategy, run it through the existing Experiment path, deploy it to Alpaca
paper, and let it run.

It does not need to be good. It needs to **exist and produce observations**, because every day
without a deployed strategy is a day Gate 15 has no evidence, and Gate 15's evidence is a
calendar that cannot be compressed later.

Record in this file: which strategy, its declared divergence thresholds, the deployment date,
and the first week's realised-vs-expected comparison.

**Files:** none — an operations task with its result written here.
**Acceptance:** the strategy is running on paper; `forward_observation` rows accumulate daily;
Gate 15 renders a real verdict for it within its first observation window; the deployment date
is recorded here and on the coverage surface as the date the forward clock started.
**Traces:** ROADMAP §1, §6 (the agent is not on the critical path to first evidence); SPEC §12.3.

---

## Phase exit

- Paper fills produce comparable forward observations automatically.
- Gate 15 renders a real verdict, stating its count and window.
- Divergence halts before the next order, proven by a drill, with no bypass.
- At least one strategy has been running on paper long enough to have produced observations,
  and the date it started is recorded.
