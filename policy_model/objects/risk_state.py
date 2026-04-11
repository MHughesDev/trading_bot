"""
RiskState for policy envelope — human policy spec §8.4.

**Note:** This is the policy-layer constraint object from the spec, not `app.contracts.risk.RiskState`
(system modes). Bridge: `policy_model.bridge.policy_envelope_from_app_settings`.
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class RiskState:
    max_abs_position_fraction: float
    max_position_delta_per_step: float
    max_leverage: float
    min_trade_notional: float
    cooldown_steps_remaining: int
    allow_long: bool
    allow_short: bool
    kill_switch_active: bool
    max_drawdown_limit: float
    concentration_limit: float
    volatility_limit: float
    daily_loss_limit_remaining: float
