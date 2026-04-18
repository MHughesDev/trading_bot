from __future__ import annotations

import pytest

from app.contracts.canonical_state import CanonicalStateOutput, DegradationLevel
from data_plane.features.canonical_normalize import (
    normalize_feature_row,
    validate_normalized_row,
)
from decision_engine.state_engine import apply_normalization_degradation


def test_normalize_aliases_ret_to_return():
    row = normalize_feature_row(
        {"close": 100.0, "ret_1": 0.01, "volume": 1.0, "rsi_14": 50.0, "atr_14": 1.0}
    )
    assert row["return_1"] == pytest.approx(0.01)
    assert "feature_freshness" in row


def test_stale_bar_lowers_freshness():
    row = normalize_feature_row(
        {"close": 100.0, "ret_1": 0.0, "volume": 1.0, "rsi_14": 50.0, "atr_14": 1.0},
        bar_age_seconds=120.0,
        stale_data_seconds=120.0,
    )
    assert row["feature_freshness"] == 0.0


def test_validate_low_confidence():
    row = normalize_feature_row({"close": 100.0})
    ok, reasons = validate_normalized_row(row)
    assert ok is False
    assert "incomplete_canonical_snapshot" in reasons


def test_apply_normalization_degradation():
    apex = CanonicalStateOutput(
        regime_probabilities=[0.2, 0.2, 0.2, 0.2, 0.2],
        regime_confidence=0.5,
        transition_probability=0.1,
        novelty=0.1,
        heat_score=0.2,
        reflexivity_score=0.2,
        degradation=DegradationLevel.NORMAL,
    )
    # Thin row + stale bar → low confidence & completeness after normalize (FB-CAN-016 rules)
    feats = normalize_feature_row({"close": 100.0}, bar_age_seconds=500.0, stale_data_seconds=120.0)
    assert feats["signal_confidence_aggregate"] < 0.25
    apex2 = apply_normalization_degradation(apex, feats)
    assert apex2.degradation == DegradationLevel.REDUCED
