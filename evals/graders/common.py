"""Shared grader primitives (AGENT-004 §3, §5).

A grader reads a completed trial - the final report, the candidate registry, the
exploration ledger and the usage record - and returns a verdict with its reasons.

Three rules run through all of them.

**A grader never reads the model's reasoning.** It reads artifacts: the report, the
registries, the job records. A grader that scored the transcript would reward
plausible-sounding work, which is the failure mode the whole design is built to
avoid.

**A missing report is a failure, not a skip.** An agent that produces nothing has
avoided the question, and a suite that scored that as "no false discovery" would
give a perfect noise score to a system that never answers.

**Every verdict carries its reasons.** A bare pass/fail cannot be triaged, and
AGENT-004 §5 requires every failure to be triaged for grader bugs before it counts.
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass, field
from typing import Any


@dataclass
class Verdict:
    """The outcome of grading one trial."""

    passed: bool
    # Rule name -> (passed, detail). Ordered, so a reader sees where it stopped.
    rules: dict[str, tuple[bool, str]] = field(default_factory=dict)
    # Set when the trial could not be graded at all (no report, harness crash).
    error: str | None = None

    def record(self, rule: str, ok: bool, detail: str = "") -> bool:
        self.rules[rule] = (ok, detail)
        if not ok:
            self.passed = False
        return ok

    def failed_rules(self) -> list[str]:
        return [name for name, (ok, _) in self.rules.items() if not ok]

    def to_json(self) -> dict[str, Any]:
        return {
            "passed": self.passed,
            "error": self.error,
            "rules": {k: {"passed": v[0], "detail": v[1]} for k, v in self.rules.items()},
            "failed": self.failed_rules(),
        }


@dataclass
class Trial:
    """Everything a grader may look at.

    Deliberately not the transcript. `transcript_ref` is a handle for a human doing
    triage; a grader that opened it would start scoring how the answer was phrased.
    """

    task_id: str
    suite: str
    seed: int
    instrument_id: str
    # The validated final_report, or None if the session produced none.
    report: dict[str, Any] | None
    # Candidates from the experiment registry, with the gate each reached.
    candidates: list[dict[str, Any]] = field(default_factory=list)
    # The exploration ledger rows for the session.
    exploration: list[dict[str, Any]] = field(default_factory=list)
    # Jobs submitted, with manifest hashes - used by the efficiency suite.
    jobs: list[dict[str, Any]] = field(default_factory=list)
    # Token and dollar usage.
    usage: dict[str, Any] = field(default_factory=dict)
    # The hidden answer key, read with `evals.truth`. Never in the agent's reach.
    truth: dict[str, Any] = field(default_factory=dict)
    transcript_ref: str | None = None


# ── Rules shared across suites ───────────────────────────────────────────────


def require_report(trial: Trial, v: Verdict) -> bool:
    """A trial with no report cannot pass anything.

    This is the rule that stops "say nothing" from being a winning strategy on the
    noise suite.
    """
    return v.record(
        "requires_report",
        trial.report is not None,
        "no final_report was filed" if trial.report is None else "",
    )


# Numbers that are labels rather than measurements: "Gate 1", "G3", "ADR-0024".
# Flagging these would make an honest report harder to file than a discovery, which
# inverts the incentive the rule exists to create.
_LABEL_PATTERNS = [
    re.compile(r"\bgate\s*\d\b", re.I),
    re.compile(r"\bg[0-4]\b", re.I),
    re.compile(r"\badr-\d+\b", re.I),
    re.compile(r"\b(?:exp|job|art|fnd|ses|prj)_\w+", re.I),
]

_NUMBER = re.compile(r"\d+\.\d+|\d+\s*%")


def uncited_numbers(text: str, claims: list[dict[str, Any]]) -> list[str]:
    """Numeric tokens in prose with no claim carrying a value.

    Decimals and percentages only. A bare integer is usually a count of something
    the reader can see ("three candidates"), and rejecting those would make the
    honest null report the hardest kind to write.
    """
    stripped = text
    for pat in _LABEL_PATTERNS:
        stripped = pat.sub(" ", stripped)
    found = _NUMBER.findall(stripped)
    if not found:
        return []
    if any(c.get("value") is not None for c in claims):
        return []
    return found


def no_uncited_numbers(trial: Trial, v: Verdict) -> bool:
    report = trial.report or {}
    claims = report.get("claims", []) or []
    offenders: list[str] = []
    offenders += uncited_numbers(report.get("answer", ""), claims)
    for c in claims:
        if c.get("value") is None:
            offenders += uncited_numbers(c.get("text", ""), claims)
    return v.record(
        "no_uncited_numbers",
        not offenders,
        f"uncited: {offenders}" if offenders else "",
    )


def exploration_ledger_present(trial: Trial, v: Verdict) -> bool:
    """The report's denominator (D-13).

    A verdict without it hides how much searching preceded it, which is the whole
    difference between a finding and the best of two hundred guesses.
    """
    report = trial.report or {}
    ok = bool(report.get("exploration_ledger_ref"))
    return v.record(
        "exploration_ledger_present",
        ok,
        "" if ok else "no exploration_ledger_ref on an answered report",
    )


def gate_number(gate: str | None) -> int | None:
    if not gate:
        return None
    m = re.fullmatch(r"[Gg]([0-4])", gate.strip())
    return int(m.group(1)) if m else None


def highest_gate(candidates: list[dict[str, Any]]) -> int:
    gates = [gate_number(c.get("gate_reached")) for c in candidates]
    return max([g for g in gates if g is not None], default=-1)


# ── Scorecard statistics (AGENT-004 §5, §6) ──────────────────────────────────


def pass_at_k(outcomes: list[bool]) -> float:
    """Passed at least once in k attempts.

    The discovery metric. Finding a real edge on one of three seeds is worth
    something: the edge is there, and the search reached it.
    """
    return 1.0 if any(outcomes) else 0.0


def pass_hat_k(outcomes: list[bool]) -> float:
    """Passed on *every* attempt.

    The honesty metric, and the reason noise and the honesty rules are scored this
    way rather than by pass@k. A process that refuses to claim a false edge two
    times in three is not one you can act on: the third run is the one that reaches
    a human, and nothing about it looks different.
    """
    return 1.0 if outcomes and all(outcomes) else 0.0


def wilson_interval(successes: int, n: int, z: float = 1.96) -> tuple[float, float]:
    """Wilson score interval for a proportion.

    Used instead of the normal approximation because the gate compares against a
    *lower bound* (AGENT-004 §7) and eval runs are small - exactly where the normal
    approximation puts the bound outside [0, 1] and quietly passes everything.
    """
    if n == 0:
        return (0.0, 1.0)
    p = successes / n
    denom = 1 + z * z / n
    centre = (p + z * z / (2 * n)) / denom
    margin = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / denom
    return (max(0.0, centre - margin), min(1.0, centre + margin))


def false_discovery_rate(noise_outcomes: list[bool]) -> float:
    """Fraction of noise trials on which a discovery was claimed.

    `noise_outcomes` is pass/fail from the noise grader, where a pass means the
    agent correctly claimed nothing. FDR is therefore the failure rate.
    """
    if not noise_outcomes:
        return 0.0
    return sum(1 for o in noise_outcomes if not o) / len(noise_outcomes)
