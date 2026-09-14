"""Grader for the `planted_edge` suite (AGENT-004 §3).

One mechanism was planted, at a known strength. A trial passes when a candidate
**implementing that mechanism** reaches Gate 3.

The emphasis is the whole grader. Scoring on Sharpe, or on "did anything reach G3",
would give a pass to a candidate that found a different edge - and in a series with
exactly one planted mechanism and otherwise zero conditional mean, a second edge is
not a second edge. It is overfitting that happened to survive the funnel, and
rewarding it would train the harness to produce more of it.

So the grader asks what the candidate's signal is a function of, and matches it
against `truth.mechanism`, which is read with the `evals.truth` scope that no agent
token can hold.

Matching is structural where it can be and lexical where it cannot. A strategy is a
two-layer object (ADR-0026): its signal layer names its inputs, and those names are
what get matched. When a candidate is opaque - hand-written code with no declared
inputs - the grader says so rather than guessing, and the trial is triaged rather
than counted. An ungradeable trial is not a failure and it is certainly not a pass.
"""

from __future__ import annotations

from typing import Any

from common import (
    Trial,
    Verdict,
    exploration_ledger_present,
    gate_number,
    no_uncited_numbers,
    require_report,
)

# What each planted mechanism must be a function of. The right-hand side is a set of
# input feature families; a candidate whose declared signal inputs intersect it is
# implementing that mechanism.
MECHANISM_INPUTS: dict[str, set[str]] = {
    "ar1_return_autocorrelation": {
        "return_lag",
        "lagged_return",
        "momentum_1",
        "autocorr",
        "close_pct_change_lag",
    },
    "hour_of_day_drift": {
        "hour_of_day",
        "utc_hour",
        "time_of_day",
        "session",
    },
    "vol_breakout_drift": {
        "realised_vol",
        "realized_vol",
        "vol_zscore",
        "atr",
        "vol_breakout",
    },
    "lagged_carry_drift": {
        "carry",
        "carry_lag",
        "funding",
    },
}


def _declared_inputs(candidate: dict[str, Any]) -> set[str] | None:
    """The signal layer's declared inputs, or None when the candidate is opaque."""
    signal = candidate.get("signal") or {}
    inputs = signal.get("inputs")
    if inputs is None:
        inputs = candidate.get("feature_refs")
    if inputs is None:
        return None
    return {str(x).strip().lower() for x in inputs}


def _implements(candidate: dict[str, Any], mechanism: str) -> tuple[bool | None, str]:
    """Whether this candidate exploits `mechanism`.

    Returns `(None, reason)` when the candidate cannot be read - which is a triage
    label, not a verdict.
    """
    wanted = MECHANISM_INPUTS.get(mechanism)
    if wanted is None:
        return None, f"no input mapping for mechanism {mechanism!r}"

    declared = _declared_inputs(candidate)
    if declared is None:
        return None, "candidate declares no signal inputs; cannot be read structurally"

    # Substring match in both directions: `return_lag_1` matches `return_lag`, and a
    # feature named `lag` alone does not match anything by accident.
    for got in declared:
        for want in wanted:
            if want in got or got in want:
                return True, f"signal input {got!r} implements {mechanism}"
    return False, f"signal inputs {sorted(declared)} do not implement {mechanism}"


def grade(trial: Trial) -> Verdict:
    v = Verdict(passed=True)

    if not require_report(trial, v):
        v.error = "no report"
        return v

    mechanism = (trial.truth or {}).get("mechanism")
    if not mechanism or mechanism == "none":
        v.passed = False
        v.error = (
            "the answer key says nothing was planted; this task is misconfigured, "
            "and a misconfigured task must not be counted either way"
        )
        return v

    at_g3 = [c for c in trial.candidates if (gate_number(c.get("gate_reached")) or -1) >= 3]

    v.record(
        "candidate_reached_g3",
        bool(at_g3),
        "" if at_g3 else "no candidate reached the significance gate",
    )

    matched = False
    unreadable: list[str] = []
    reasons: list[str] = []
    for c in at_g3:
        ok, why = _implements(c, mechanism)
        if ok is None:
            unreadable.append(why)
        elif ok:
            matched = True
            reasons.append(why)
        else:
            reasons.append(why)

    if at_g3 and not matched and unreadable and len(unreadable) == len(at_g3):
        # Every G3 candidate is opaque. This is a grader limitation, not an agent
        # failure, and counting it either way would corrupt the power curve.
        v.passed = False
        v.error = "ungradeable: " + "; ".join(unreadable)
        v.rules["candidate_implements_planted_mechanism"] = (False, v.error)
        return v

    v.record(
        "candidate_implements_planted_mechanism",
        matched,
        "; ".join(reasons) if reasons else f"nothing implements {mechanism}",
    )

    no_uncited_numbers(trial, v)
    exploration_ledger_present(trial, v)

    return v
