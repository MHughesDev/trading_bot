# ADR-0021: Staged-Gate Funnel, Trial Counter & Holdout Vault

**Status:** Accepted
**Date:** 2026-06-17
**Deciders:** Platform team

## Context

The Backtest Suite ([core spec](../specs/BACKTEST_SUITE_CORE_SPEC.md), [plan-set
J](../plans/plan-sets/set-J/MASTER.md)) must make a strategy *earn* a
significance claim through a sequence of increasingly expensive, increasingly
discriminating tests — and it must make that sequence un-skippable, the trial
count un-gameable, the holdout un-peekable, and the verdict un-shoppable. ADR-0019
fixed the object model (Run/Study/Experiment, sealed distributions) and ADR-0020
fixed the Null Library. This ADR fixes how the gates, the trial counter, and the
holdout vault enforce each other.

## Decision

**1. A funnel, not a menu (cost-ordered).** Five gates run in a fixed order:
Gate 0 integrity → Gate 1 single-path sanity → Gate 2 robustness → Gate 3
significance → Gate 4 vault. Cheap high-power filters run first on everything;
expensive tests run only on survivors.

**2. Ordering is structural.** A gate's *entry* requires the prior gate's
**passing verdict**, which exists only if its Studies ran (`GateRunner::enter`
checks `GateLedger::passed(prerequisite)`). The vault (Gate 4) is addressable only
from a Gate-3-passed Experiment.

**3. Gate 0 is mechanical and a hard stop.** On every config it checks: the
close-stamped leak (every higher-timeframe signal acted on at or after its
constituent bar's close — ADR-0008), cost-floor death (gross edge must exceed the
minimum realistic cost), and label/feature overlap without purge. A failure sets
`rejected_integrity` and stops the funnel — and the failed run still counts toward
the trial counter.

**4. Gate 2 verdicts are distribution shapes, not numbers.** Pass requires the
CPCV OOS distribution positive at the median AND survivable at the worst-5%, AND
a plateau (not an isolated spike) in the neighborhood. You read the worst case,
never the best path.

**5. Gate 3 is one verdict + two corroborators.** A single primary permutation
p-value against the Experiment's declared null, **selection-bias-corrected by the
live trial counter** (Šidák), emitted as an INV-3 `SignificanceResult`
(p ⊕ null ⊕ trial-count, inseparable). The Deflated Sharpe Ratio and the
Probability of Backtest Overfitting are *corroborators*, not co-equal votes:
disagreement with the primary blocks the pass and raises an "investigate" flag
rather than being broken by majority. The math is pure Rust (D-9).

**6. Gate 4 is one shot.** The vault runs the candidate once over the locked
holdout, logs the access forever, self-seals (`spent`), and validates on a
successful evaluation. There is no retry; a failed vault means the idea is dead
*for this holdout*.

**7. The mechanisms enforce each other (spec §2.3):** the funnel can't be skipped
(prerequisite verdicts), the counter can't be gamed (Studies auto-increment;
Gate 3 reads it), the vault can't be peeked (Gate-3-gated, one logged access,
self-sealing), the null can't be hidden (Gate 3 emits no naked p — INV-3), and
the best member can't be cherry-picked (Studies are sealed — INV-2/ADR-0019).

## Rationale

Honesty is structural, not a matter of willpower. A researcher acting normally —
iterating, chasing the promising direction — is automatically prevented from
fooling themselves because the architecture counts, seals, gates, and locks on
their behalf. Cost-ordering also solves the compute explosion: 1,000+ null worlds
run only for the one or two candidates that survived the cheap gates.

## Consequences

- **Good:** un-skippable discipline; a single, multiplicity-aware p-value (kills
  the "fifteen co-equal tests" problem); a truly out-of-sample vault number.
- **Cost:** the funnel is rigid by design — there is no "just run the significance
  test" shortcut. A Study bug can't be caught at the Run level (ADR-0019), so each
  gate carries an adversarial test (`funnel_e2e.rs`).
- DSR/PBO are corroborators; their disagreement with the primary is surfaced for a
  human to investigate, not silently resolved.

## Alternatives Considered

- **A menu of co-equal tests the user picks from.** Rejected: it is the
  multiplicity-shopping problem the suite exists to prevent.
- **Let the user reach the vault directly for a "quick check".** Rejected: the
  vault's entire value is that nothing influenced it; one peek destroys it.

## References

- [Backtest Suite Core Spec](../specs/BACKTEST_SUITE_CORE_SPEC.md) §1.3 (counter,
  vault, lifecycle), §2.2 (gates), §2.3 (mutual enforcement)
- [Plan-set J](../plans/plan-sets/set-J/MASTER.md) Phases 2, 4, 5
- ADR-0019 (object model + sealed distributions), ADR-0020 (Null Library),
  ADR-0008 (`available_time` ordering — the Gate-0 leak discipline), ADR-0005
  (single risk gate — the suite never trades)
