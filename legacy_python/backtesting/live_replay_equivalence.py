"""Live–replay deterministic equivalence helpers (FB-CAN-030).

Compares canonical ``decision_output_event`` payloads across replay runs or against saved
fingerprints. Used by CI and release evidence; see APEX Replay spec §7 and Config Gating spec.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from typing import Any

# Policy default: zero mismatched decision fingerprints allowed for promotion checks.
DEFAULT_DIVERGENCE_MISMATCH_MAX: int = 0


def _stable_json(obj: Any) -> str:
    return json.dumps(obj, sort_keys=True, default=str, separators=(",", ":"))


def fingerprint_decision_output_event(event_dict: dict[str, Any]) -> str:
    """SHA-256 of the decision output ``payload`` only (stable key order)."""
    payload = event_dict.get("payload")
    if not isinstance(payload, dict):
        payload = {}
    return hashlib.sha256(_stable_json(payload).encode("utf-8")).hexdigest()


def extract_decision_output_fingerprints(canonical_events: list[dict[str, Any]]) -> list[str]:
    """One fingerprint per ``decision_output_event`` in order."""
    out: list[str] = []
    for ev in canonical_events:
        if not isinstance(ev, dict):
            continue
        if ev.get("event_family") == "decision_output_event":
            out.append(fingerprint_decision_output_event(ev))
    return out


@dataclass
class LiveReplayEquivalenceReport:
    """Result of comparing two replay runs or a run vs baseline fingerprints."""

    equivalent: bool
    bars_compared: int
    mismatch_count: int
    mismatch_indices: list[int] = field(default_factory=list)
    left_hash: str | None = None
    right_hash: str | None = None
    notes: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "equivalent": self.equivalent,
            "bars_compared": self.bars_compared,
            "mismatch_count": self.mismatch_count,
            "mismatch_indices": self.mismatch_indices,
            "left_hash": self.left_hash,
            "right_hash": self.right_hash,
            "notes": self.notes,
        }


def compare_decision_fingerprint_sequences(
    left: list[str],
    right: list[str],
    *,
    mismatch_max: int = DEFAULT_DIVERGENCE_MISMATCH_MAX,
) -> LiveReplayEquivalenceReport:
    """Compare two per-bar decision fingerprint lists (e.g. two replays of the same contract)."""
    if len(left) != len(right):
        lh = hashlib.sha256(_stable_json(left).encode()).hexdigest() if left else None
        rh = hashlib.sha256(_stable_json(right).encode()).hexdigest() if right else None
        return LiveReplayEquivalenceReport(
            equivalent=False,
            bars_compared=max(len(left), len(right)),
            mismatch_count=max(len(left), len(right)),
            mismatch_indices=list(
                range(min(len(left), len(right)), max(len(left), len(right)))
            )[:32],
            left_hash=lh,
            right_hash=rh,
            notes="sequence length mismatch",
        )
    mismatches = [i for i in range(len(left)) if left[i] != right[i]]
    mc = len(mismatches)
    lh = hashlib.sha256(_stable_json(left).encode()).hexdigest() if left else None
    rh = hashlib.sha256(_stable_json(right).encode()).hexdigest() if right else None
    return LiveReplayEquivalenceReport(
        equivalent=mc <= mismatch_max,
        bars_compared=len(left),
        mismatch_count=mc,
        mismatch_indices=mismatches[:32],
        left_hash=lh,
        right_hash=rh,
        notes="",
    )


def fingerprints_from_replay_rows(rows: list[dict[str, Any]]) -> list[str]:
    """Collect decision fingerprints from each row's ``canonical_events`` (one per bar)."""
    fps: list[str] = []
    for row in rows:
        evs = row.get("canonical_events")
        if not isinstance(evs, list):
            continue
        fp = extract_decision_output_fingerprints(evs)
        if len(fp) == 1:
            fps.append(fp[0])
        elif len(fp) == 0:
            fps.append("")
        else:
            # Multiple decision events in one bar — concatenate stable hashes
            fps.append(hashlib.sha256(_stable_json(fp).encode()).hexdigest())
    return fps


__all__ = [
    "DEFAULT_DIVERGENCE_MISMATCH_MAX",
    "LiveReplayEquivalenceReport",
    "compare_decision_fingerprint_sequences",
    "extract_decision_output_fingerprints",
    "fingerprints_from_replay_rows",
    "fingerprint_decision_output_event",
]
