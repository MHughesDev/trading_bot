"""FB-CAN-030 live–replay equivalence helpers."""

from __future__ import annotations

from backtesting.live_replay_equivalence import (
    compare_decision_fingerprint_sequences,
    extract_decision_output_fingerprints,
    fingerprint_decision_output_event,
)


def test_fingerprint_stable_for_payload():
    ev = {
        "event_family": "decision_output_event",
        "payload": {"route": {"a": 1}, "z": 2},
    }
    a = fingerprint_decision_output_event(ev)
    b = fingerprint_decision_output_event(dict(ev))
    assert a == b


def test_compare_sequences():
    left = ["a", "b", "c"]
    assert compare_decision_fingerprint_sequences(left, list(left)).equivalent is True
    assert compare_decision_fingerprint_sequences(left, ["a", "x", "c"]).equivalent is False


def test_extract_decision_fingerprints_filters_family():
    evs = [
        {"event_family": "market_snapshot_event", "payload": {}},
        {"event_family": "decision_output_event", "payload": {"x": 1}},
    ]
    fps = extract_decision_output_fingerprints(evs)
    assert len(fps) == 1
