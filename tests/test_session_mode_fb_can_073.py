"""FB-CAN-073: weekend / low-liquidity session mode and risk occupancy."""

from __future__ import annotations

from datetime import UTC, datetime

from app.config.settings import AppSettings
from app.contracts.canonical_state import DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from decision_engine.state_engine import (
    build_canonical_state,
    classify_session_mode,
    merge_canonical_into_risk,
)


def _pkt(ts: datetime) -> ForecastPacket:
    return ForecastPacket(
        timestamp=ts,
        horizons=[1, 3, 5],
        q_low=[-0.01, -0.02, -0.03],
        q_med=[0.0, 0.0, 0.0],
        q_high=[0.01, 0.02, 0.03],
        interval_width=[0.02, 0.04, 0.06],
        regime_vector=[0.4, 0.3, 0.2, 0.1],
        confidence_score=0.8,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.1,
    )


def test_classify_session_mode_weekend():
    sat = datetime(2026, 4, 18, 12, 0, 0, tzinfo=UTC)
    mode, thr, codes = classify_session_mode(
        data_timestamp=sat,
        feature_row={"close": 1.0, "depth_near_touch": 1.0},
        settings=AppSettings(),
    )
    assert mode == "weekend"
    assert thr < 1.0
    assert any("weekend" in c for c in codes)


def test_classify_session_mode_low_liquidity_overrides_weekend():
    sat = datetime(2026, 4, 18, 12, 0, 0, tzinfo=UTC)
    mode, thr, codes = classify_session_mode(
        data_timestamp=sat,
        feature_row={"close": 1.0, "depth_near_touch": 0.1},
        settings=AppSettings(),
    )
    assert mode == "low_liquidity"
    assert thr < 1.0


def test_build_canonical_state_sets_session_fields():
    ts = datetime(2026, 4, 19, 10, 0, 0, tzinfo=UTC)  # Sunday
    feats = {"close": 50_000.0, "atr_14": 100.0, "rsi_14": 50.0, "depth_near_touch": 1.0}
    apex = build_canonical_state(_pkt(ts), feats, spread_bps=5.0, data_timestamp=ts)
    assert apex.session_mode == "weekend"
    assert apex.session_mode_throttle < 1.0


def test_merge_canonical_tracks_session_occupancy():
    ts = datetime(2026, 4, 19, 10, 0, 0, tzinfo=UTC)
    feats = {"close": 50_000.0, "atr_14": 100.0, "rsi_14": 50.0, "depth_near_touch": 1.0}
    apex = build_canonical_state(_pkt(ts), feats, spread_bps=5.0, data_timestamp=ts)
    r0 = RiskState()
    r1 = merge_canonical_into_risk(r0, apex, settings=AppSettings())
    assert r1.session_mode == "weekend"
    assert r1.session_mode_occupancy_ticks.get("weekend", 0) >= 1
    r2 = merge_canonical_into_risk(r1, apex, settings=AppSettings())
    assert r2.session_mode_occupancy_ticks.get("weekend", 0) >= 2


def test_session_throttle_reduces_composite_multiplier_when_not_no_trade():
    ts = datetime(2026, 4, 14, 12, 0, 0, tzinfo=UTC)  # weekday
    feats = {
        "close": 50_000.0,
        "atr_14": 100.0,
        "rsi_14": 50.0,
        "depth_near_touch": 0.1,
    }
    apex = build_canonical_state(_pkt(ts), feats, spread_bps=5.0, data_timestamp=ts)
    assert apex.session_mode == "low_liquidity"
    assert apex.degradation != DegradationLevel.NO_TRADE
    r0 = RiskState()
    r1 = merge_canonical_into_risk(r0, apex, settings=AppSettings())
    terms = r1.canonical_degradation_sizing_terms or {}
    assert terms.get("session_mode_throttle", 1.0) < 1.0
    assert r1.canonical_size_multiplier <= terms.get("degradation_base_multiplier", 1.0) + 1e-9
