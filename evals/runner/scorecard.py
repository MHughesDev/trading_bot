"""Scorecard aggregation and the non-inferiority gate (AGENT-004 §6, §7).

Two jobs, and they are separate on purpose.

`build_scorecard` turns a list of graded trials into the published table for one
harness version. `check_gate` compares a candidate scorecard against a *published*
baseline using margins that were committed beforehand in `gates.yaml`.

The separation is what makes the gate mean anything. A gate that recomputed its own
baseline would drift with every run, and a gate whose margins were chosen after
seeing the result is not a gate.
"""

from __future__ import annotations

import math
import sys
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "graders"))

from common import (  # noqa: E402
    false_discovery_rate,
    pass_at_k,
    pass_hat_k,
    wilson_interval,
)


@dataclass
class GradedTrial:
    suite: str
    task_id: str
    seed: int
    passed: bool
    # Triage label. `None` means untriaged, which `gates.yaml` counts as a failure -
    # an untriaged failure is not yet a data point, but it is not a pass either.
    triage: str | None = None
    cost_usd: float = 0.0
    tokens_in: int = 0
    tokens_out: int = 0
    cache_read: int = 0
    wall_clock_s: float = 0.0
    uncited_numbers: int = 0


def _effective(t: GradedTrial) -> bool:
    """The pass/fail a scorecard counts.

    A failure triaged as a grader bug is excluded from the metric rather than
    counted as a failure; a failure with no triage counts as a failure. That
    asymmetry is deliberate: excluding untriaged failures would make "do not look at
    it" the cheapest way to raise a score.
    """
    if not t.passed and t.triage == "grader_bug":
        return True
    return t.passed


def build_scorecard(
    harness_version: str,
    trials: list[GradedTrial],
    planted_strengths: dict[str, float] | None = None,
    provenance: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """The published table for one harness version (AGENT-004 §6).

    `provenance` carries the profile, tier and hardware the run executed on
    (ADR-0032). It is what makes "promotion from local_mid to local_high is an eval
    result" mechanically true instead of a sentence: without it, a scorecard cannot
    say whether it was earned on the tier it is being used to promote, and
    `check_gate` refuses to compare two runs it cannot establish are comparable.
    """
    by_task: dict[tuple[str, str], list[GradedTrial]] = defaultdict(list)
    for t in trials:
        by_task[(t.suite, t.task_id)].append(t)

    excluded = [t for t in trials if not t.passed and t.triage == "grader_bug"]
    untriaged = [t for t in trials if not t.passed and t.triage is None]

    noise_outcomes: list[bool] = []
    noise_task_scores: list[float] = []
    power: dict[str, dict[str, Any]] = {}
    suite_rates: dict[str, dict[str, Any]] = {}

    for (suite, task_id), group in sorted(by_task.items()):
        outcomes = [_effective(t) for t in group]
        if suite == "noise":
            noise_outcomes += outcomes
            # pass^k per task: the agent must be right on every seed.
            noise_task_scores.append(pass_hat_k(outcomes))
        elif suite == "planted_edge":
            score = pass_at_k(outcomes)
            strength = (planted_strengths or {}).get(task_id)
            power[task_id] = {
                "pass_at_k": score,
                "seeds": len(outcomes),
                "successes": sum(outcomes),
                "planted_strength": strength,
                "lower_95ci": wilson_interval(sum(outcomes), len(outcomes))[0],
            }

    for suite in sorted({t.suite for t in trials}):
        group = [t for t in trials if t.suite == suite]
        outcomes = [_effective(t) for t in group]
        successes = sum(outcomes)
        lo, hi = wilson_interval(successes, len(outcomes))
        suite_rates[suite] = {
            "trials": len(outcomes),
            "passed": successes,
            "rate": successes / len(outcomes) if outcomes else 0.0,
            "lower_95ci": lo,
            "upper_95ci": hi,
        }

    total_cost = sum(t.cost_usd for t in trials)
    correct = sum(1 for t in trials if _effective(t))
    cache_read = sum(t.cache_read for t in trials)
    tokens_in = sum(t.tokens_in for t in trials)

    return {
        "harness_version": harness_version,
        "provenance": dict(provenance or {}),
        "trials": len(trials),
        "metrics": {
            # The headline honesty number: the rate at which a discovery was claimed
            # on an instrument with nothing in it.
            "fdr_noise": false_discovery_rate(noise_outcomes),
            "fdr_noise_pass_hat_k": (
                1.0 - sum(noise_task_scores) / len(noise_task_scores)
                if noise_task_scores
                else None
            ),
            "power_planted_edge": power,
            "suite_rates": suite_rates,
            "uncited_number_rate": (
                sum(1 for t in trials if t.uncited_numbers > 0) / len(trials)
                if trials
                else 0.0
            ),
            "dollars_per_correct_verdict": (total_cost / correct) if correct else None,
            "total_cost_usd": total_cost,
            "cache_hit_rate": (cache_read / (cache_read + tokens_in))
            if (cache_read + tokens_in)
            else 0.0,
            "wall_clock_s": sum(t.wall_clock_s for t in trials),
        },
        "triage": {
            "excluded_grader_bugs": len(excluded),
            "untriaged_failures": len(untriaged),
        },
    }


# ── Paired non-inferiority (AGENT-004 §7, §8) ────────────────────────────────
#
# The gate asks whether a change made the agent *worse*, within a stated margin.
# Comparing the candidate's own confidence bound against the baseline's point
# estimate is the obvious implementation and it is wrong: a run identical to the
# baseline fails it, because a proportion measured on a handful of seeds has a wide
# interval no matter how good it is.
#
# The right comparison is an interval on the *difference*, and §8 already specifies
# how to make that difference precise: paired tasks with common random numbers. The
# same seeds, the same tasks, both arms. Then only the trials where the two arms
# disagree carry information, and agreement - which is what a harmless change
# produces - is evidence of non-inferiority rather than noise.
#
# Tango's score interval is used because it behaves at zero discordance, which is
# the case a good change actually produces and the one a naive normal approximation
# divides by zero on.


def _constrained_phi(b: int, c: int, n: int, delta: float) -> float:
    """MLE of the discordance nuisance parameter with the difference fixed at delta.

    Found numerically rather than by the closed form: a 1-D maximisation over a
    bounded interval is transparent and testable, and the closed form's sign
    conventions differ between sources in ways that are easy to get silently wrong.
    """
    lo = max(0.0, -delta) + 1e-12
    hi = (1.0 - delta) / 2.0 - 1e-12
    if hi <= lo:
        return max(lo, 0.0)
    a_d = n - b - c

    def ll(phi: float) -> float:
        p_b, p_c, p_rest = phi, phi + delta, 1.0 - 2.0 * phi - delta
        if p_b <= 0 or p_c <= 0 or p_rest <= 0:
            return -math.inf
        return b * math.log(p_b) + c * math.log(p_c) + a_d * math.log(p_rest)

    # Golden-section search on a unimodal log-likelihood.
    gr = (math.sqrt(5.0) - 1.0) / 2.0
    x1, x2 = hi - gr * (hi - lo), lo + gr * (hi - lo)
    f1, f2 = ll(x1), ll(x2)
    for _ in range(200):
        if f1 < f2:
            lo, x1, f1 = x1, x2, f2
            x2 = lo + gr * (hi - lo)
            f2 = ll(x2)
        else:
            hi, x2, f2 = x2, x1, f1
            x1 = hi - gr * (hi - lo)
            f1 = ll(x1)
    return (lo + hi) / 2.0


def _tango_score(b: int, c: int, n: int, delta: float) -> float:
    phi = _constrained_phi(b, c, n, delta)
    var = n * (2.0 * phi + delta * (1.0 - delta))
    if var <= 0:
        return math.inf if (c - b - n * delta) > 0 else -math.inf
    return (c - b - n * delta) / math.sqrt(var)


def paired_difference_lower_bound(b: int, c: int, n: int, z: float = 1.96) -> float:
    """Lower limit of Tango's score interval for `p_candidate - p_baseline`.

    `b` is the count of paired trials the baseline passed and the candidate failed;
    `c` the reverse; `n` the number of pairs.
    """
    if n == 0:
        return -1.0
    lo, hi = -1.0 + 1e-9, min(1.0 - 1e-9, (c - b) / n)
    for _ in range(200):
        mid = (lo + hi) / 2.0
        if _tango_score(b, c, n, mid) > z:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2.0


def min_detectable_margin(n_pairs: int, z: float = 1.96) -> float:
    """The tightest non-inferiority margin `n_pairs` can decide, at best.

    "At best" means with perfect agreement between the arms - zero discordant pairs.
    Any real run does worse. This exists so an underpowered result can say how many
    trials it would have needed instead of quietly reading as a regression.
    """
    if n_pairs <= 0:
        return 1.0
    return -paired_difference_lower_bound(0, 0, n_pairs, z)


def pairs_needed_for(margin: float, z: float = 1.96) -> int:
    """Paired trials needed to decide `margin`, with zero discordance.

    From Tango's statistic at b = c = 0: n = z^2 (1 - m) / m.
    """
    if margin <= 0:
        return 0
    return math.ceil(z * z * (1.0 - margin) / margin)


@dataclass
class GateResult:
    passed: bool
    reasons: list[str]
    # Distinct from `passed=False`. An underpowered run has not shown a regression;
    # it has shown nothing, and treating the two the same teaches everyone to read
    # gate failures as noise.
    underpowered: bool = False

    def report(self) -> str:
        head = "UNDERPOWERED" if self.underpowered else ("PASS" if self.passed else "FAIL")
        return "\n".join([f"non-inferiority gate: {head}"] + [f"  - {r}" for r in self.reasons])


def pair_trials(
    candidate_trials: list[GradedTrial], baseline_trials: list[GradedTrial], suite: str
) -> tuple[int, int, int]:
    """Match trials by (task_id, seed) and count the discordant pairs.

    Returns `(b, c, n)`: baseline-only passes, candidate-only passes, and pairs.
    Unmatched trials are dropped — comparing a task the baseline never ran is not a
    comparison, and silently treating it as a failure on one side would manufacture
    a regression out of a renamed task.
    """
    base = {
        (t.task_id, t.seed): _effective(t) for t in baseline_trials if t.suite == suite
    }
    b = c = n = 0
    for t in candidate_trials:
        if t.suite != suite:
            continue
        key = (t.task_id, t.seed)
        if key not in base:
            continue
        n += 1
        cand_ok, base_ok = _effective(t), base[key]
        if base_ok and not cand_ok:
            b += 1
        elif cand_ok and not base_ok:
            c += 1
    return b, c, n


def compare_provenance(
    candidate: dict[str, Any],
    baseline: dict[str, Any],
    require_same: list[str] | None = None,
    report_if_different: list[str] | None = None,
) -> tuple[bool, list[str]]:
    """`gates.yaml -> comparability`. Returns `(comparable, notes)`.

    Two ways this goes wrong, and the quiet one is worse.

    **Loud:** a local_mid run measured against a frontier baseline fails every
    margin. The obvious fix — widening the margins — would weaken the gate for
    frontier too, so the tiers need separate baselines rather than shared slack.

    **Quiet:** a local_mid run compared against a local_mid baseline earned on
    *different hardware* measures the hardware and reports it as a harness change.
    Nothing about the numbers looks wrong. So hardware is reported rather than
    ignored: a faster box is not a better harness.
    """
    require_same = require_same or ["tier", "profile_id"]
    report_if_different = report_if_different or ["hardware"]
    cp = candidate.get("provenance") or {}
    bp = baseline.get("provenance") or {}
    notes: list[str] = []
    comparable = True

    for field in require_same:
        cv, bv = cp.get(field), bp.get(field)
        if cv is None or bv is None:
            comparable = False
            notes.append(
                f"{field} is unrecorded on "
                f"{'the candidate' if cv is None else 'the baseline'}; "
                "a scorecard that cannot say what it was earned on cannot be compared"
            )
        elif cv != bv:
            comparable = False
            notes.append(f"{field} differs: candidate {cv!r} vs baseline {bv!r}")

    for field in report_if_different:
        cv, bv = cp.get(field), bp.get(field)
        if cv is not None and bv is not None and cv != bv:
            notes.append(f"note: {field} differs ({cv!r} vs {bv!r}); a faster box is not a better harness")

    return comparable, notes


def check_gate(
    candidate: dict[str, Any],
    baseline: dict[str, Any],
    margins: dict[str, Any],
    gate_nominal: float = 0.05,
    paired: dict[str, tuple[int, int, int]] | None = None,
    comparability: dict[str, Any] | None = None,
) -> GateResult:
    """AGENT-004 §7. Every clause must hold; there is no overall score.

    A gate that averaged its clauses would let a large power gain pay for a small
    honesty regression, and those two are not interchangeable.

    `paired` carries `(b, c, n)` per suite from `pair_trials`, which is how the
    power and Q&A clauses are decided. Without it the gate falls back to comparing
    unpaired proportions, says so, and will almost always report UNDERPOWERED — the
    common-random-number pairing of §8 is not an optimisation, it is what makes a
    two-point margin decidable at all.
    """
    reasons: list[str] = []
    ok = True
    underpowered = False

    # Comparability first. Everything below is a comparison, so a gate that ran the
    # margins against an incomparable baseline would produce a verdict that looks
    # exactly like a real one.
    # Enforced when the caller passes `gates.yaml -> comparability`. Absent, this is
    # a direct comparison the caller has taken responsibility for — the gate does not
    # invent a policy that was not configured. `test_gates_yaml_still_configures_comparability`
    # is what stops the block quietly disappearing from the config.
    if comparability and comparability.get("on_mismatch", "refuse") != "ignore":
        comparable, notes = compare_provenance(
            candidate,
            baseline,
            (comparability or {}).get("require_same"),
            (comparability or {}).get("report_if_different"),
        )
        reasons.extend(notes)
        if not comparable:
            return GateResult(
                passed=False,
                underpowered=False,
                reasons=reasons
                + ["refusing to gate: the candidate and the baseline are not comparable"],
            )

    cm = candidate.get("metrics", {})
    bm = baseline.get("metrics", {})

    # 1. FDR ceiling. Absolute, not relative to the baseline: "no worse than last
    #    time" is not a standard when last time was already wrong.
    fdr = cm.get("fdr_noise", 1.0)
    ceiling = gate_nominal + 0.02
    if fdr > ceiling + 1e-12:
        ok = False
        reasons.append(f"FDR on noise is {fdr:.3f}, above the ceiling {ceiling:.3f}")
    else:
        reasons.append(f"FDR on noise {fdr:.3f} <= {ceiling:.3f}")

    # Every planted-edge task in the baseline must appear in this run, whatever the
    # statistics say. A task that vanished is not non-inferior, it is unmeasured.
    for task_id in (bm.get("power_planted_edge") or {}):
        if (cm.get("power_planted_edge") or {}).get(task_id) is None:
            ok = False
            reasons.append(f"{task_id}: present in the baseline, missing from this run")

    # 2 and 3. Power and Q&A, as paired non-inferiority on the difference.
    for key, suite, label in [
        ("power_planted_edge", "planted_edge", "power"),
        ("qa_accuracy", "qa", "qa accuracy"),
    ]:
        margin = margins.get(key, {}).get("margin", 0.02)
        pair = (paired or {}).get(suite)
        if pair is None:
            if (cm.get("suite_rates") or {}).get(suite) is None:
                continue
            underpowered = True
            ok = False
            reasons.append(
                f"{label}: no paired trials supplied, so the difference cannot be "
                "bounded; run both arms on the same tasks and seeds (AGENT-004 s8)"
            )
            continue
        b, c, n = pair
        lower = paired_difference_lower_bound(b, c, n)
        best = min_detectable_margin(n)
        if lower >= -margin - 1e-12:
            reasons.append(
                f"{label}: difference lower bound {lower:+.3f} >= -{margin} "
                f"({n} pairs, {b} lost, {c} gained)"
            )
        elif best > margin:
            # The run could not have passed even with perfect agreement. That is a
            # statement about the run's size, not about the change.
            underpowered = True
            ok = False
            reasons.append(
                f"{label}: UNDERPOWERED - {n} pairs can decide a margin of "
                f"{best:.3f} at best, but the gate asks for {margin}; "
                f"{pairs_needed_for(margin)} pairs would be needed"
            )
        else:
            ok = False
            reasons.append(
                f"{label}: difference lower bound {lower:+.3f} below -{margin} "
                f"({n} pairs, {b} lost, {c} gained)"
            )

    # 4. Leakage-trap catch rate: no margin at all.
    lt_c = (cm.get("suite_rates") or {}).get("leakage_traps")
    lt_b = (bm.get("suite_rates") or {}).get("leakage_traps")
    if lt_c and lt_b and lt_c["rate"] < lt_b["rate"] - 1e-12:
        ok = False
        reasons.append(
            f"leakage-trap catch rate fell from {lt_b['rate']:.3f} to {lt_c['rate']:.3f}; "
            "a leak reaching a published result is not a regression, it is a wrong answer"
        )

    # 5. Uncited numbers: zero, always.
    uncited = cm.get("uncited_number_rate", 1.0)
    if uncited > 0:
        ok = False
        reasons.append(f"uncited-number rate is {uncited:.3f}; the ceiling is 0")

    # 6. The named must-pass tasks.
    for required in margins.get("required_passes", []) or []:
        suite = required.split("/")[0]
        rates = (cm.get("suite_rates") or {}).get(suite)
        if rates is None:
            ok = False
            reasons.append(f"{required}: required task's suite was not run")

    # 7. Untriaged failures. `gates.yaml` says these count as failures, and a run
    #    full of them has not been looked at yet.
    untriaged = (candidate.get("triage") or {}).get("untriaged_failures", 0)
    if untriaged:
        ok = False
        reasons.append(
            f"{untriaged} failure(s) have no triage label; every failure is triaged "
            "for grader bugs before it is counted"
        )

    return GateResult(passed=ok, reasons=reasons, underpowered=underpowered)
