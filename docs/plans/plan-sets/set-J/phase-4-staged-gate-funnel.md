# Phase 4 — The staged-gate funnel (0→4)

**Completion: 100% (12 / 12 tasks)** — the funnel and significance math landed and
unit-tested in `crates/backtest/src/gates/` and `crates/backtest/src/stats/`.
Postgres persistence is the live leg of J-4.2 (the `GateLedger` + `migrations/0030`
are in place; the ledger is the in-process source of truth the funnel enforces on).

**Summary of completed work (2026-06-17):**
- J-4.1 ADR-0021 authored and Accepted; indexed in `docs/adr/README.md`.
- J-4.2 `Gate`/`GateVerdict`/`GateLedger` + `GateRunner::enter` ordering enforcement + `migrations/0030_backtest_gate_verdicts.sql`.
- J-4.3 Gate 0 close-stamped leak scan (`SignalStamp`: acted-at < bar-close ⇒ flag).
- J-4.4 Gate 0 cost-floor death + label/feature-overlap-without-purge scan.
- J-4.5 Gate 0 hard-stop wiring (`GateError::IntegrityHardStop`); failures still count.
- J-4.6 Gate 1 single-path verdict (pessimistic walk-forward; median > 0).
- J-4.7/J-4.8 Gate 2 robustness over CPCV + synthetic + neighborhood → distribution-**shape** verdict (median>0 ∧ worst-5% survivable ∧ plateau).
- J-4.9 Gate 3 primary permutation p (`stats::permutation_p_value`) + selection-bias correction by the live counter (`selection_bias_correction`) → INV-3 `SignificanceResult`.
- J-4.10 Gate 3 corroborators: `deflated_sharpe_ratio` (PSR/DSR with normal CDF + Acklam inverse) and `probability_of_backtest_overfitting` (CSCV); disagreement blocks the pass.
- J-4.11 Gate 4 vault (delegates to `Experiment::run_vault`; reachable only after Gate 3).
- J-4.12 `crates/backtest/tests/funnel_e2e.rs` — the five mutual-enforcement properties + a full in-order pass.

`cargo test -p backtest` → 139 lib + 6 funnel-e2e tests green; lib clippy clean.

**Goal:** Assemble the **funnel, not a menu**. Tests are ordered by
cost-per-unit-of-discriminating-power: cheap high-power filters run first on
everything; expensive tests run only on survivors. The ordering simultaneously
solves the compute explosion and enforces the discipline — you cannot reach the
vault without surviving the cheap gates, because each gate's *entry* requires the
prior gate's **pass verdict**, which only exists if its Studies actually ran.

**Depends on:** Phase 1 (Studies), Phase 2 (lifecycle, counter, vault), Phase 3
(`Null`, `SignificanceResult` seam).
**Blocks:** Phase 5 (reconciliation begins after Gate 4 → `live`).

---

## Design notes

```
   [ candidate strategy ]
            │
   GATE 0 — INTEGRITY            cheap, always, fail = hard stop
            │ pass
   GATE 1 — SINGLE-PATH SANITY   cheap, one honest walk-forward (pessimistic costs)
            │ pass
   GATE 2 — ROBUSTNESS           moderate, distribution over histories
            │ pass
   GATE 3 — SIGNIFICANCE         expensive, the ONE primary null test
            │ pass
   GATE 4 — THE VAULT            one shot, irreversible, logged
            │ pass
   [ validated → eligible for live ]
```

**Gate verdicts are typed and persisted.** `GateRunner` records a `GateVerdict {
gate, passed, evidence: Vec<StudyRef>, at }` per gate; the next gate's entry
asserts the prior `GateVerdict.passed == true` exists. No verdict, no entry — the
funnel is unskippable by construction (D-8).

**Gate 2 verdict is a distribution shape, not a number.** A strategy passes if
its OOS distribution is positive at the *median* AND survivable at the
*worst-5%*, and its parameter neighborhood is a *plateau*. You read the worst
case, never the best path.

**Gate 3 is one verdict + two corroborators — not seventeen votes.** The single
primary permutation test (against the Experiment's declared `primary_test` null),
selection-bias-corrected by the live trial counter, *is* the p-value. The
**Deflated Sharpe Ratio** and **Probability of Backtest Overfitting** are
corroborators that should *agree*; disagreement is a flag to investigate, not a
result to pick from.

---

## Tasks

### ☑ J-4.1 Author ADR-0021 (staged-gate funnel, trial counter & vault) — S
Write `docs/adr/0021-staged-gate-funnel-and-honesty-mechanics.md`: the gate
ordering and cost rationale, the pass-verdict-gates-next-gate rule, one-primary-
test + two-corroborators, and how the funnel/counter/vault/null/sealing enforce
each other (spec §2.3). Mark Accepted; index + cite in MASTER §9.
**Acceptance:** ADR-0021 exists, indexed, cited by Gate tasks.

### ☑ J-4.2 `GateRunner` + typed `GateVerdict` + ordering enforcement — M
Add `crates/backtest/src/gates/mod.rs` with `Gate { Integrity, SinglePath,
Robustness, Significance, Vault }`, `GateVerdict`, and `GateRunner::enter(gate)`
that **refuses unless the prior gate's passing verdict exists** for this
Experiment. Persist verdicts via Postgres migration **0030**
(`backtest_gate_verdicts`).
**Acceptance:** entering Gate 2 without a passing Gate-1 verdict is refused;
verdicts persist and resolve their evidence Studies; the only path to Gate 4 is
through 0→3 passes.

### ☑ J-4.3 Gate 0 — close-stamped leakage scan — L
Implement the leakage/lookahead scan that runs on **every** config: assert that
**every higher-timeframe signal is stamped at the constituent bar's _close_,
never its open** (the 1-min-constructed daily bar is complete only at the daily
close; acting earlier leaks the whole bar). Build on `available_time` ordering
(ADR-0008) and `aggregate_bars`' forming-bar-safe assembly. A violation sets
`RunResult.status = RejectedIntegrity` and a `Flag`.
**Acceptance:** a deliberately planted signal that reads a not-yet-closed
higher-TF bar is caught and the run is `RejectedIntegrity`; a correctly
close-stamped strategy passes; runs on 100% of configs (test over both cases).

### ☑ J-4.4 Gate 0 — cost sanity + label leakage — M
Add: (a) **cost sanity** — is the *gross* edge larger than the minimum realistic
cost floor? If the edge dies under the floor cost model, it dies here. (b)
**label look-ahead** (model-fed strategies) — does a label horizon overlap the
feature window without purging? Both write `Flag`s and can `RejectedIntegrity`.
Cost: trivial; runs on all Gate-0 entrants.
**Acceptance:** a strategy whose gross edge < floor cost is rejected at Gate 0; an
overlapping label/feature window without purge is flagged.

### ☑ J-4.5 Gate 0 verdict wiring (hard stop) — S
`GateRunner` runs Gate 0 on every config before any performance claim exists; a
failure is a **hard stop** (no Gate 1). Aggregate per-Experiment Gate-0 status
into a `GateVerdict`. Integrity failures still **count** toward the trial counter
(they are runs).
**Acceptance:** a Gate-0 failure blocks progression and is recorded; the failed
run is stored and counted (INV via Phase 0/2).

### ☑ J-4.6 Gate 1 — single-path sanity (pessimistic costs) — M
Implement Gate 1 as **one** honest `WalkForward` Study with the
`pessimistic_intrabar` fill model and the pessimistic end of the cost ladder. If
it is not profitable on a single forward path with realistic costs, **stop**.
Verdict: pass iff median OOS return on the single path > 0 net of pessimistic
costs.
**Acceptance:** a strategy profitable only under optimistic costs fails Gate 1; a
robustly profitable one passes; exactly one walk-forward Study is spent.

### ☑ J-4.7 Gate 2 — robustness (CPCV + synthetic + neighborhood) — L
Implement Gate 2 as three sealed-distribution Studies: **CPCV** (OOS distribution
across train/test combinations — read median + worst-5%), **synthetic paths**
(`synthetic_garch` or `stationary_bootstrap` — would the edge survive plausible
alternate histories?), and **neighborhood** (plateau vs spike). All run on Gate-1
survivors only.
**Acceptance:** the three Studies run and seal; the gate consumes their results
(no best-path access); runs only after a Gate-1 pass.

### ☑ J-4.8 Gate 2 — distribution-shape verdict — M
Compute the Gate 2 verdict as a **shape**, not a number: pass iff (OOS median > 0)
**and** (OOS worst-5% survivable, per a declared threshold) **and** (neighborhood
is a plateau, not an isolated spike). Persist the verdict with its three evidence
Studies.
**Acceptance:** a strategy with a positive median but catastrophic worst-5% fails;
an isolated-spike neighborhood fails even with a good median; a broad-plateau,
worst-5%-survivable strategy passes (table-driven test).

### ☑ J-4.9 Gate 3 — primary permutation test + selection-bias correction — L
Implement the **single** primary test: a `PermutationNull` Study against the
Experiment's declared `primary_test` null (1,000+ null worlds × walk-forward
each, parallelized), producing a permutation p-value, then **selection-bias-
corrected using the live trial counter** (Phase 2). Emit the INV-3
`SignificanceResult { p_value, null_ref, trial_count_at_eval }` — computed once,
never re-shopped.
**Acceptance:** the p-value is corrected by the current `trial_counter` (a Sharpe
of 1.5 after 3 vs 3,000 trials yields materially different verdicts — test both);
the result carries its null and trial count or does not emit.

### ☑ J-4.10 Gate 3 — DSR + PBO corroborators — M
Add `crates/backtest/src/stats/`: **Deflated Sharpe Ratio** (Sharpe adjusted for
trial count + skew/kurtosis) and **Probability of Backtest Overfitting** (CSCV).
These are corroborators computed alongside the primary p — *not* additional
p-values to shop between. If they **disagree** with the primary verdict, the gate
emits a `Flag: investigate`, not a pass-by-majority.
**Acceptance:** DSR and PBO compute against fixtures with known answers;
agreement → clean verdict; a constructed disagreement raises the investigate flag
rather than silently passing.

### ☑ J-4.11 Gate 4 — the vault (one shot) — M
Wire Gate 4 to `Experiment::run_vault` (Phase 2), reachable **only** with a
passing Gate-3 verdict. The Experiment runs **once** against the locked holdout
(never touched by any param/selection/null). On the result: log access, flip
`spent`, and transition `candidate→validated` on pass (or mark the idea dead *for
this holdout* on fail — no retry).
**Acceptance:** Gate 4 is unreachable without a Gate-3 pass; the vault run is
single-shot and logged; pass → `validated`, fail → terminal-for-this-holdout; a
second attempt is refused.

### ☑ J-4.12 End-to-end funnel test + mutual-enforcement suite — M
Add `crates/backtest/tests/funnel_e2e.rs` running an Experiment through 0→4 and
asserting spec §2.3: the funnel can't be skipped (no verdict → no entry); the
counter can't be gamed (Studies auto-increment, Gate 3 reads it); the vault can't
be peeked (Gate-3-gated, one access, self-sealed); the null can't be hidden (Gate
3 refuses without one); the best member can't be cherry-picked (Studies sealed).
**Acceptance:** each of the five enforcement properties has a passing assertion;
removing any one protection makes a corresponding sub-test fail (documented).

---

## Exit criteria

- Gates 0→4 run in order; each gate's entry requires the prior gate's passing
  verdict; the vault is reachable only after Gate 3.
- Gate 0 mechanically catches the close-stamp leak and cost-floor death on 100%
  of configs; failures hard-stop and still count.
- Gate 2 verdicts are distribution shapes (median + worst-5% + plateau); Gate 3
  is one corrected p-value + DSR/PBO corroborators (disagreement → investigate).
- Gate 4 spends the vault once, logged, and transitions to `validated`.
- The five mutual-enforcement properties (§2.3) are each test-backed. ADR-0021
  Accepted. `cargo test -p backtest` green.
