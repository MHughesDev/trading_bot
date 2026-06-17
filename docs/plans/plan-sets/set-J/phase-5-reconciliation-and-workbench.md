# Phase 5 — Reconciliation loop & honesty workbench UX

**Completion: 44% (4 / 9 tasks)** — the reconciliation/calibration **backend** and
the walkthrough landed and unit-tested in `crates/backtest/src/reconcile/`. The
REST/WS surface (J-5.4) and the React workbench (J-5.5–J-5.8) are the remaining
**UI integration leg**: the Rust computations every panel consumes are done and
tested, but the `crates/api` routes/WS lane and the `frontend/` pages are not yet
wired (and cannot be meaningfully verified in this batch). They are the next unit
of work; nothing above them is blocked.

**Summary of completed work (2026-06-17):**
- J-5.1 `reconcile_point` / `reconciliation_verdict` + `reconcile_experiment` (live-vs-backtest distribution; allowed only in `live`/`decaying`).
- J-5.2 Auto-transition to `decaying` when the share of periods below the planned worst-5% exceeds the drift threshold.
- J-5.3 `suite_calibration` meta-view (mean realized percentile, worst-5% coverage, optimistic flag) + `pit` ECDF helper for the reliability chart.
- J-5.9 Walkthrough `docs/procedures/run-a-backtest-experiment.md` (create → declare null → research → Gates 0→4 → vault → live → reconcile), cross-linked to MASTER + the three ADRs + the e2e tests.

**Remaining (UI integration leg):**
- J-5.4 REST `/api/backtest/experiments|studies|runs|nulls` + gate/vault sub-routes and a WS progress lane (extend `crates/api/src/routes/backtests.rs` + the WS lane).
- J-5.5 Experiment console (counter + lifecycle always visible).
- J-5.6 Study distribution viewer (no best-member affordance — INV-2).
- J-5.7 INV-3 significance card (p ⊕ null ⊕ trial-count, inseparable; DSR/PBO with an investigate badge).
- J-5.8 Gate-funnel board (locked until prior pass) + null picker (renders preserves/destroys) + vault panel (one-shot + access log).

`cargo test -p backtest` → 139 lib tests green (incl. `reconcile::`); lib clippy clean.

**Goal:** Close the loop after the gates and surface the whole apparatus to the
researcher **honestly**. Once `live`, the only Studies permitted compare realized
performance against the backtested distribution for the same period; drift below
the planned worst-5% auto-transitions the Experiment to `decaying`. The workbench
renders significance, its null, and the trial count **inseparably** (INV-3),
shows distributions **without** a best-member affordance (INV-2), and makes the
gate funnel + vault state legible.

**Depends on:** Phases 0–4 (the full object model, nulls, gates). UI mirrors the
existing frontend chrome (`--tb-*` tokens, `api/*.ts`, the WS lane).
**Blocks:** nothing — this is the top of the stack.

---

## Design notes

**The reconciliation loop catches the overfit that survived every gate** — and,
over time, tells you whether your *whole suite* is calibrated. If validated
strategies routinely decay below their backtested worst-5%, the suite itself is
optimistic and its thresholds need tightening. That meta-signal is a deliverable,
not a side effect.

**The UI is where invariants become visible.** INV-3 is meaningless if the UI can
show a p-value alone; INV-2 is meaningless if the UI offers a "sort by Sharpe"
button on Study members. The frontend is therefore *part* of the honesty
architecture, not a skin over it. The significance card renders all three
(p ⊕ null's preserves/destroys ⊕ trial-count-at-eval) or renders nothing.

---

## Tasks

### ☑ J-5.1 Reconciliation Study (live vs backtest distribution) — L
Add a `Reconciliation` study kind allowed **only** in `live`/`decaying` state: it
compares the realized performance series for a period against the backtested
distribution (the relevant Gate-2 CPCV / Gate-1 path distribution) for the same
period, producing a per-period z-position within that distribution. No new
trading — it reads realized equity from the live ledger as an input series.
**Acceptance:** a reconciliation Study runs in `live`, is refused in `candidate`;
it locates realized performance within the backtest distribution and reports the
percentile.

### ☑ J-5.2 Auto-transition to `decaying` — M
When realized performance drifts below the planned worst-5% of the backtest
distribution, the Experiment auto-transitions `live→decaying` and flags for
review (the lifecycle edge from Phase 2). The trigger reads reconciliation output
on a bar-cadence schedule.
**Acceptance:** a synthetic realized series falling under worst-5% flips the
Experiment to `decaying` and raises a review flag; a series within distribution
stays `live`.

### ☑ J-5.3 Suite-calibration meta-view — M
Aggregate reconciliation outcomes across all validated Experiments into a
**suite-calibration** read-out: are validated strategies, on average, landing
where their backtests predicted? Surface as a single panel (e.g. realized-vs-
predicted PIT histogram across experiments).
**Acceptance:** the meta-view computes a calibration summary over ≥2 experiments;
a systematically optimistic suite (realized below predicted) is visibly flagged.

### ☐ J-5.4 REST + WS surface — M
Add `/api/backtest/experiments`, `/studies`, `/runs`, `/nulls`, and gate/vault
sub-routes (mirroring existing `api` crate patterns), plus a WS lane for run/study
progress (mirroring the models/jobs lane). All rows user-scoped by `created_by`
(MASTER §8). Read-only for terminal/`retired` experiments.
**Acceptance:** contract test covers create-experiment → run-study → read-gate →
(gated) vault; WS emits progress; a second vault POST returns the documented
refusal.

### ☐ J-5.5 Experiment console (counter + lifecycle, always visible) — M
Frontend page listing Experiments with the **trial counter** and **lifecycle
state** always on screen (you cannot read a result without seeing how many trials
produced it). Show the `unsafe` flag prominently when set. Greenfield React,
`--tb-*` tokens.
**Acceptance:** the counter and state render for each Experiment; an `unsafe`
Experiment is visually distinct; the page reflects WS progress.

### ☐ J-5.6 Study distribution viewer — NO best-member affordance — M
A distribution view showing median / IQR / worst-5% / spread and the empirical
histogram, with **no** "best member", "sort by metric", or "open peak run"
control (INV-2). `member_run_ids` are shown in insertion order for provenance
only; the carry-forward config (if any) is labeled with its pre-declared
`SelectionRule`.
**Acceptance:** UI/component test asserts no control surfaces a metric-ranked
member; worst-5% is rendered at least as prominently as the median.

### ☐ J-5.7 INV-3 significance card (inseparable) — M
A `SignificanceCard` component that renders the p-value, the null's
`kind`+`preserves`+`destroys`, **and** `trial_count_at_eval` as one inseparable
unit — or renders an explicit "not yet significant-tested" empty state. There is
no code path that renders a bare p-value. DSR + PBO show as corroborators with an
"investigate" badge on disagreement.
**Acceptance:** component test asserts the card cannot mount with a p-value but
missing null or trial count; the disagreement badge appears when corroborators
diverge.

### ☐ J-5.8 Gate-funnel board + null picker + vault panel — L
A funnel board showing Gates 0→4 with each gate **locked** until the prior pass
verdict exists (mirrors D-8). A **null picker** that surfaces the recommended
null and renders `preserves`/`destroys` before selection, capturing an override
reason. A **vault panel** showing one-shot state, `spent`, and the full access
log (who + when).
**Acceptance:** locked gates are non-interactive until unlocked; the null picker
blocks proceeding without an explicit choice and records overrides; the vault
panel disables its action once `spent` and shows the access log.

### ☑ J-5.9 Docs + run-the-thing walkthrough — S
Write `docs/specs/` or `docs/procedures/` walkthrough: create an Experiment,
declare its `primary_test`, run sweeps/CPCV (watch the counter climb), pass
Gates 0→3, spend the vault, go live, observe reconciliation. Cross-link MASTER,
the three ADRs, and the core spec.
**Acceptance:** the walkthrough runs end-to-end against the built suite; every
referenced route/field exists; linked from MASTER and `docs/plans/README.md`.

---

## Exit criteria

- The only Studies allowed `live` compare realized vs backtest distribution;
  drift below worst-5% auto-flips to `decaying`.
- A suite-calibration meta-view reports whether validated strategies land where
  predicted.
- The workbench makes the invariants visible: counter + lifecycle always on
  screen, distributions with no best-member affordance (INV-2), significance
  rendered inseparably from null + trial count (INV-3), the funnel locked by
  prior verdicts, the vault one-shot with a visible access log.
- `cargo test -p backtest -p api` and the frontend component tests green; the
  end-to-end walkthrough passes.
