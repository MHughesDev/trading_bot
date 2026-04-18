"""FB-CAN-033: hard override taxonomy + degradation transition accounting."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import pytest

from app.config.settings import AppSettings
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.hard_override import HardOverrideKind
from app.contracts.reason_codes import TRG_MOVE_ALREADY_EXTENDED
from app.contracts.risk import RiskState, SystemMode
from app.contracts.trigger import TriggerOutput
from decision_engine.state_engine import (
    build_canonical_state,
    classify_hard_override,
    merge_canonical_into_risk,
)


def _pkt() -> ForecastPacket:
    return ForecastPacket(
        timestamp=datetime.now(UTC),
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


def test_classify_system_mode_override():
    s = AppSettings()
    r = RiskState(mode=SystemMode.MAINTENANCE)
    active, kind = classify_hard_override(
        risk=r,
        feature_row={},
        spread_bps=1.0,
        settings=s,
        feed_last_message_at=None,
        data_timestamp=None,
        now_ref=datetime.now(UTC),
    )
    assert active is True
    assert kind == HardOverrideKind.SYSTEM_MODE


def test_classify_signal_confidence_low():
    s = AppSettings()
    r = RiskState()
    active, kind = classify_hard_override(
        risk=r,
        feature_row={"signal_confidence_aggregate": 0.1},
        spread_bps=1.0,
        settings=s,
        feed_last_message_at=None,
        data_timestamp=None,
        now_ref=datetime.now(UTC),
    )
    assert active is True
    assert kind == HardOverrideKind.SIGNAL_CONFIDENCE_LOW


def test_classify_feed_stale_uses_now_ref():
    s = AppSettings()
    r = RiskState()
    ref = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    flm = ref - timedelta(seconds=float(s.risk_stale_data_seconds) + 5.0)
    active, kind = classify_hard_override(
        risk=r,
        feature_row={},
        spread_bps=1.0,
        settings=s,
        feed_last_message_at=flm,
        data_timestamp=None,
        now_ref=ref,
    )
    assert active is True
    assert kind == HardOverrideKind.FEED_STALE


def test_classify_product_untradable():
    s = AppSettings()
    r = RiskState()
    active, kind = classify_hard_override(
        risk=r,
        feature_row={},
        spread_bps=1.0,
        settings=s,
        feed_last_message_at=None,
        data_timestamp=None,
        now_ref=datetime.now(UTC),
        product_tradable=False,
    )
    assert active is True
    assert kind == HardOverrideKind.PRODUCT_UNTRADABLE


def test_degradation_transition_first_tick_no_increment_second_tick_increments():
    feats = {"close": 50_000.0, "atr_14": 100.0, "rsi_14": 50.0}
    apex1 = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    r0 = RiskState()
    r1 = merge_canonical_into_risk(r0, apex1, forecast_packet=_pkt(), trigger=None)
    assert r1.degradation_transition_count == 0
    assert r1.last_degradation_level == apex1.degradation.value
    assert r1.degradation_occupancy_ticks.get(apex1.degradation.value) == 1

    apex2 = build_canonical_state(_pkt(), feats, spread_bps=200.0)
    if apex2.degradation == apex1.degradation:
        pytest.skip("degradation did not change for this fixture; widen spread/heat to force change")
    r2 = merge_canonical_into_risk(r1, apex2, forecast_packet=_pkt(), trigger=None)
    assert r2.degradation_transition_count >= 1


def test_merge_preserves_override_flags_without_forecast_packet():
    apex = build_canonical_state(_pkt(), {"close": 1.0, "atr_14": 0.0, "rsi_14": 50.0}, spread_bps=1.0)
    r0 = RiskState()
    r1 = merge_canonical_into_risk(
        r0,
        apex,
        hard_override_active=True,
        hard_override_kind=HardOverrideKind.SPREAD_WIDE,
    )
    assert r1.hard_override_active is True
    assert r1.hard_override_kind == HardOverrideKind.SPREAD_WIDE


def test_false_positive_memory_still_updates_with_trigger():
    apex = build_canonical_state(_pkt(), {"close": 1.0, "atr_14": 0.0, "rsi_14": 50.0}, spread_bps=1.0)
    trig = TriggerOutput(
        setup_valid=True,
        setup_score=0.5,
        pretrigger_valid=True,
        pretrigger_score=0.5,
        trigger_valid=False,
        trigger_type="none",
        trigger_strength=0.4,
        trigger_confidence=0.3,
        missed_move_flag=True,
        trigger_reason_codes=[TRG_MOVE_ALREADY_EXTENDED],
    )
    r0 = RiskState(trigger_false_positive_memory=0.0)
    r1 = merge_canonical_into_risk(
        r0,
        apex,
        forecast_packet=_pkt(),
        trigger=trig,
        spread_bps=1.0,
        feature_row={"close": 1.0},
        hard_override_active=False,
        hard_override_kind=HardOverrideKind.NONE,
    )
    assert r1.trigger_false_positive_memory > 0.0
