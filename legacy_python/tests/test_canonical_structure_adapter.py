"""Tests for CanonicalStructureOutput + structure_from_forecast_packet (FB-CAN-017)."""

from __future__ import annotations

from datetime import UTC, datetime

from app.contracts.forecast_packet import ForecastPacket
from app.contracts.structure_adapter import structure_from_forecast_packet


def _packet(**kwargs) -> ForecastPacket:
    defaults = dict(
        timestamp=datetime.now(UTC),
        horizons=[1, 2, 3],
        q_low=[-0.02, -0.01, 0.0],
        q_med=[0.01, 0.02, 0.03],
        q_high=[0.04, 0.05, 0.06],
        interval_width=[0.06, 0.06, 0.06],
        regime_vector=[0.2, 0.2, 0.2, 0.2, 0.2],
        confidence_score=0.8,
        ensemble_variance=[1.0, 1.1, 0.9],
        ood_score=0.1,
        forecast_diagnostics={},
    )
    defaults.update(kwargs)
    return ForecastPacket(**defaults)


def test_structure_percentiles_and_asymmetry():
    st = structure_from_forecast_packet(_packet())
    assert st.p05 == -0.02
    assert st.p95 == 0.04
    assert 0.0 <= st.asymmetry_score <= 1.0
    assert -1.0 <= st.directional_bias <= 1.0


def test_empty_packet_structure_is_degenerate():
    st = structure_from_forecast_packet(
        ForecastPacket(
            timestamp=datetime.now(UTC),
            horizons=[],
            q_low=[],
            q_med=[],
            q_high=[],
            interval_width=[],
            regime_vector=[],
            confidence_score=0.0,
            ensemble_variance=[],
            ood_score=1.0,
        )
    )
    assert st.fragility_score >= 0.9


def test_oi_structure_class_from_diagnostics():
    pkt = _packet(forecast_diagnostics={"oi_structure_class": "crowded_long"})
    st = structure_from_forecast_packet(pkt)
    assert st.oi_structure_class == "crowded_long"
