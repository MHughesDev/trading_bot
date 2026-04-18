"""FB-CAN-074: exchange risk + data integrity in degradation, overrides, and sizing."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from app.config.settings import AppSettings
from app.contracts.canonical_state import DegradationLevel
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.hard_override import HardOverrideKind
from app.contracts.reason_codes import (
    STATE_DATA_INTEGRITY_ALERT,
    STATE_EXCHANGE_RISK_CRITICAL,
    STATE_EXCHANGE_RISK_ELEVATED,
    STATE_EXCHANGE_RISK_HIGH,
)
from app.contracts.risk import RiskState, SystemMode
from decision_engine.state_engine import (
    apply_normalization_degradation,
    build_canonical_state,
    classify_exchange_risk_and_integrity,
    classify_hard_override,
    composite_degradation_size_multiplier,
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
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=0.8,
        ensemble_variance=[0.01, 0.02, 0.03],
        ood_score=0.1,
    )


def test_classify_exchange_risk_and_integrity_codes():
    fr = {"apex_exchange_risk_level_code": 1.0, "apex_data_integrity_alert": 0.0}
    lvl, di, codes = classify_exchange_risk_and_integrity(fr)
    assert lvl == "elevated"
    assert di is False
    assert STATE_EXCHANGE_RISK_ELEVATED in codes

    fr2 = {"apex_exchange_risk_level_code": 3.0, "apex_data_integrity_alert": 1.0}
    lvl2, di2, codes2 = classify_exchange_risk_and_integrity(fr2)
    assert lvl2 == "critical"
    assert di2 is True
    assert STATE_DATA_INTEGRITY_ALERT in codes2
    assert STATE_EXCHANGE_RISK_CRITICAL in codes2


def test_build_canonical_state_bumps_degradation_for_high_risk():
    feats = {
        "close": 1.0,
        "atr_14": 0.01,
        "rsi_14": 50.0,
        "apex_exchange_risk_level_code": 2.0,
    }
    apex = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    assert apex.degradation == DegradationLevel.DEFENSIVE
    assert apex.exchange_risk_level == "high"
    assert STATE_EXCHANGE_RISK_HIGH in apex.safety_reason_codes


def test_data_integrity_alert_no_trade():
    feats = {
        "close": 1.0,
        "atr_14": 0.01,
        "rsi_14": 50.0,
        "apex_data_integrity_alert": 1.0,
    }
    apex = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    assert apex.degradation == DegradationLevel.NO_TRADE
    assert apex.data_integrity_alert is True


def test_hard_override_data_integrity_and_critical_exchange():
    risk = RiskState(mode=SystemMode.RUNNING)
    settings = AppSettings()
    now = datetime(2026, 4, 18, 12, 0, tzinfo=UTC)
    ho1, k1 = classify_hard_override(
        risk=risk,
        feature_row={"apex_data_integrity_alert": 1.0},
        spread_bps=1.0,
        settings=settings,
        feed_last_message_at=now,
        data_timestamp=now,
        now_ref=now,
    )
    assert ho1 is True
    assert k1 == HardOverrideKind.DATA_INTEGRITY_ALERT

    ho2, k2 = classify_hard_override(
        risk=risk,
        feature_row={"apex_exchange_risk_level_code": 3.0},
        spread_bps=1.0,
        settings=settings,
        feed_last_message_at=now,
        data_timestamp=now,
        now_ref=now,
    )
    assert ho2 is True
    assert k2 == HardOverrideKind.EXCHANGE_RISK_CRITICAL


def test_composite_exchange_risk_throttle():
    feats = {
        "close": 1.0,
        "atr_14": 0.01,
        "rsi_14": 50.0,
        "apex_exchange_risk_level_code": 1.0,
    }
    apex = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    settings = AppSettings()
    m, terms = composite_degradation_size_multiplier(apex.degradation, apex, settings)
    assert terms["exchange_risk_throttle"] < 1.0
    assert m > 0.0


def test_merge_canonical_into_risk_sets_exchange_fields():
    feats = {
        "close": 1.0,
        "atr_14": 0.01,
        "rsi_14": 50.0,
        "apex_exchange_risk_level_code": 1.0,
    }
    apex = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    r0 = RiskState(mode=SystemMode.RUNNING)
    r1 = merge_canonical_into_risk(r0, apex, settings=AppSettings())
    assert r1.exchange_risk_level == "elevated"
    assert r1.data_integrity_alert is False


def test_apply_normalization_degradation_exchange_risk():
    feats = {
        "close": 1.0,
        "atr_14": 0.01,
        "rsi_14": 50.0,
        "apex_exchange_risk_level_code": 1.0,
    }
    apex0 = build_canonical_state(_pkt(), feats, spread_bps=5.0)
    # Force normal so normalization bump is visible
    apex_n = apex0.model_copy(update={"degradation": DegradationLevel.NORMAL})
    out = apply_normalization_degradation(apex_n, feats)
    assert out.degradation == DegradationLevel.REDUCED


if __name__ == "__main__":
    pytest.main([__file__, "-q"])
