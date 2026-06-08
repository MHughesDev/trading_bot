"""FB-CAN-032 canonical signal confidence families."""

from __future__ import annotations

from app.config.signal_confidence import (
    REQUIRED_SIGNAL_FAMILIES,
    apply_signal_family_confidence,
    validate_signal_confidence_domain,
)


def _minimal_signal_confidence() -> dict:
    base = {
        "enabled": True,
        "base_confidence_floor": 0.0,
        "base_confidence_cap": 1.0,
        "freshness_floor": 0.0,
        "freshness_cap": 1.0,
        "decay_lambda": 1.0,
        "latency_penalty_weight": 0.1,
        "reliability_penalty_weight": 0.1,
    }
    return {name: dict(base) for name in REQUIRED_SIGNAL_FAMILIES}


def test_validate_requires_all_families():
    d = _minimal_signal_confidence()
    del d["funding"]
    errs = validate_signal_confidence_domain(d)
    assert any("funding" in e for e in errs)


def test_validate_rejects_cap_below_floor():
    d = _minimal_signal_confidence()
    d["funding"]["base_confidence_cap"] = 0.0
    d["funding"]["base_confidence_floor"] = 0.5
    errs = validate_signal_confidence_domain(d)
    assert errs


def test_apply_disabled_family_zero():
    row = {"feature_freshness": 0.9, "feature_reliability": 0.9, "close": 100.0}
    sc = _minimal_signal_confidence()
    ff = {n: {"enabled": n != "funding"} for n in REQUIRED_SIGNAL_FAMILIES}
    out = apply_signal_family_confidence(row, signal_confidence=sc, feature_families=ff)
    assert out["signal_confidence_funding"] == 0.0
    assert "signal_confidence_aggregate" in out


def test_apply_sets_per_family_keys():
    row = {"feature_freshness": 0.95, "feature_reliability": 0.9, "close": 50000.0}
    sc = _minimal_signal_confidence()
    ff = {n: {"enabled": True} for n in REQUIRED_SIGNAL_FAMILIES}
    out = apply_signal_family_confidence(row, signal_confidence=sc, feature_families=ff)
    for n in REQUIRED_SIGNAL_FAMILIES:
        assert f"signal_confidence_{n}" in out
