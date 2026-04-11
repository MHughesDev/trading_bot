"""Tests for `RiskState` → `PolicyRiskEnvelope` bridge (`policy_model.risk_bridge`)."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.risk import RiskState, SystemMode
from policy_model.risk_bridge import risk_state_to_policy_envelope


def _settings() -> AppSettings:
    return AppSettings(
        risk_max_total_exposure_usd=100_000.0,
        risk_max_per_symbol_usd=40_000.0,
        risk_max_drawdown_pct=0.15,
    )


def test_bridge_max_abs_from_settings() -> None:
    env = risk_state_to_policy_envelope(RiskState(), _settings())
    assert env.max_abs_position_fraction == 0.4
    assert env.max_drawdown_limit == 0.15
    assert env.kill_switch_active is False


def test_bridge_kill_switch_maintenance_and_flatten() -> None:
    for mode in (SystemMode.MAINTENANCE, SystemMode.FLATTEN_ALL):
        env = risk_state_to_policy_envelope(RiskState(mode=mode), _settings())
        assert env.kill_switch_active is True


def test_bridge_running_not_kill() -> None:
    env = risk_state_to_policy_envelope(RiskState(mode=SystemMode.RUNNING), _settings())
    assert env.kill_switch_active is False


def test_bridge_allow_direction_overrides() -> None:
    env = risk_state_to_policy_envelope(
        RiskState(),
        _settings(),
        allow_long=False,
        allow_short=True,
    )
    assert env.allow_long is False
    assert env.allow_short is True
