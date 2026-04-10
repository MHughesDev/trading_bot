"""policy_model.bridge: PolicyRiskEnvelope from AppSettings + RiskState."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.risk import RiskState, SystemMode
from policy_model.bridge import policy_envelope_from_app_settings


def test_bridge_running_mode() -> None:
    s = AppSettings()
    r = RiskState()
    env = policy_envelope_from_app_settings(s, r)
    assert env.max_abs_position_fraction > 0
    assert not env.kill_switch_active


def test_bridge_maintenance_kill() -> None:
    s = AppSettings()
    r = RiskState(mode=SystemMode.MAINTENANCE)
    env = policy_envelope_from_app_settings(s, r)
    assert env.kill_switch_active
