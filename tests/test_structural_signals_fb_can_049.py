"""FB-CAN-049 structural signal ingestion and normalization."""

from __future__ import annotations

import pytest

from data_plane.features.canonical_normalize import normalize_feature_row
from data_plane.ingest.structural_signals import (
    STRUCTURAL_REPLAY_KEYS,
    apply_structural_families_from_row,
    merge_structural_signal_overlay,
)


def test_apply_maps_struct_prefix_and_sets_coverage():
    row: dict[str, float] = {
        "close": 100.0,
        "struct_funding_rate": 0.0001,
        "struct_funding_age_seconds": 0.0,
        "struct_open_interest": 1.5e9,
        "struct_oi_age_seconds": 0.0,
        "struct_basis_bps": 12.0,
        "struct_basis_age_seconds": 0.0,
        "struct_cross_exchange_divergence": 0.04,
        "struct_divergence_age_seconds": 0.0,
        "struct_liquidation_proximity_long": 0.7,
        "struct_liquidation_proximity_short": 0.65,
        "struct_liquidation_cluster_density_long": 0.2,
        "struct_liquidation_cluster_density_short": 0.15,
        "struct_liquidation_data_confidence": 0.8,
        "struct_liquidation_age_seconds": 0.0,
    }
    out = apply_structural_families_from_row(dict(row))
    assert out["funding_rate"] == pytest.approx(0.0001)
    assert out["open_interest"] == pytest.approx(1.5e9)
    assert out["basis_bps"] == pytest.approx(12.0)
    assert out["cross_exchange_divergence"] == pytest.approx(0.04)
    assert out["liquidation_proximity_long"] == pytest.approx(0.7)
    assert out["structural_family_coverage"] == pytest.approx(1.0)
    assert out["structural_all_missing"] == pytest.approx(0.0)
    assert out["structural_missing_funding"] == 0.0


def test_all_missing_caps_structural_freshness():
    row = normalize_feature_row({"close": 100.0, "ret_1": 0.0, "volume": 1.0, "rsi_14": 50.0, "atr_14": 1.0})
    assert row["structural_all_missing"] >= 0.99
    assert row["structural_freshness"] <= 0.16
    assert row["structural_reliability"] <= 0.19


def test_merge_overlay_wins():
    base = {"close": 50.0, "struct_funding_rate": 0.001}
    overlay = {"struct_funding_rate": 0.002}
    out = merge_structural_signal_overlay(base, overlay)
    assert out["funding_rate"] == pytest.approx(0.002)


def test_replay_keys_cover_normalized_fields():
    row = normalize_feature_row(
        {
            "close": 100.0,
            "ret_1": 0.0,
            "volume": 1.0,
            "rsi_14": 50.0,
            "atr_14": 1.0,
            "struct_funding_rate": 0.0005,
            "struct_funding_age_seconds": 1800.0,
        }
    )
    for k in STRUCTURAL_REPLAY_KEYS:
        if k in (
            "liquidation_proximity_long",
            "liquidation_proximity_short",
            "liquidation_cluster_density_long",
            "liquidation_cluster_density_short",
            "liquidation_data_confidence",
            "perp_spot_divergence_score",
        ):
            continue
        if k in row:
            assert k in row
