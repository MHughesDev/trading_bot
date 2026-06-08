"""FB-CAN-050 options context + stablecoin flow proxy families."""

from __future__ import annotations

import pytest

from app.config.signal_confidence import apply_signal_family_confidence
from data_plane.features.canonical_normalize import normalize_feature_row
from data_plane.features.optional_families import apply_options_and_stablecoin_families


def test_normalize_sets_availability_without_upstream_options():
    row = normalize_feature_row(
        {"close": 100.0, "ret_1": 0.0, "volume": 1.0, "rsi_14": 50.0, "atr_14": 1.0}
    )
    assert row["options_context_available"] == 0.0
    assert row["stablecoin_flow_available"] == 0.0


def test_options_fields_set_availability_and_freshness():
    base = {
        "close": 100.0,
        "ret_1": 0.0,
        "volume": 1.0,
        "rsi_14": 50.0,
        "atr_14": 1.0,
        "gex_score": 0.4,
        "struct_options_age_seconds": 0.0,
    }
    row = normalize_feature_row(base)
    assert row["options_context_available"] == 1.0
    assert row["options_freshness"] > 0.9


def test_stablecoin_proxy_and_freshness():
    base = {
        "close": 100.0,
        "ret_1": 0.0,
        "volume": 1.0,
        "rsi_14": 50.0,
        "atr_14": 1.0,
        "struct_stablecoin_flow_proxy": 0.15,
        "struct_stablecoin_age_seconds": 1800.0,
    }
    row = normalize_feature_row(base)
    assert row["stablecoin_flow_available"] == 1.0
    assert "stablecoin_flow_proxy" in row
    assert 0.0 < row["stablecoin_freshness"] < 1.0


def test_signal_confidence_fallback_when_enabled_no_data():
    sc = {
        "market_microstructure": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 1.0,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "funding": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 0.9,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "open_interest": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 0.9,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "basis": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 0.9,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "cross_exchange_divergence": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 0.9,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "liquidation_structure": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 0.9,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "options_context": {
            "enabled": True,
            "base_confidence_floor": 0.0,
            "base_confidence_cap": 0.5,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "stablecoin_flow_proxy": {
            "enabled": True,
            "base_confidence_floor": 0.0,
            "base_confidence_cap": 0.5,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "execution_feedback": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 1.0,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "novelty": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 1.0,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
        "heat_components": {
            "enabled": True,
            "base_confidence_floor": 0.1,
            "base_confidence_cap": 1.0,
            "freshness_floor": 0.0,
            "freshness_cap": 1.0,
            "decay_lambda": 0.0,
            "latency_penalty_weight": 0.0,
            "reliability_penalty_weight": 0.0,
        },
    }
    ff = {k: {"enabled": True} for k in sc}
    row = normalize_feature_row(
        {"close": 100.0, "ret_1": 0.0, "volume": 1.0, "rsi_14": 50.0, "atr_14": 1.0}
    )
    out = apply_signal_family_confidence(row, signal_confidence=sc, feature_families=ff)
    assert out["signal_confidence_options_context"] == pytest.approx(0.0)
    assert out["options_context_fallback_active"] == pytest.approx(1.0)
    assert out["signal_confidence_stablecoin_flow_proxy"] == pytest.approx(0.0)
    assert out["stablecoin_flow_proxy_fallback_active"] == pytest.approx(1.0)


def test_apply_options_idempotent():
    row: dict[str, float] = {"gex_score": 0.2}
    apply_options_and_stablecoin_families(row)
    apply_options_and_stablecoin_families(row)
    assert row["gex_score"] == pytest.approx(0.2)
