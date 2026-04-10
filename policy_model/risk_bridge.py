"""Map `app.contracts.risk.RiskState` (+ limits from `AppSettings`) to `PolicyRiskEnvelope`.

When the forecast/policy stack is wired into live or replay (`FB-PL-P1`), call this once per
decision tick so `PolicySystem` and `PolicyRiskGate` share limits with the core risk engine.

`RiskState` carries **system modes** and lightweight telemetry; `PolicyRiskEnvelope` carries
**position-style** clamps used by the policy gate. This module is the single conversion point.

Notes:

- ``MAINTENANCE`` and ``FLATTEN_ALL`` set ``kill_switch_active`` so the policy gate rejects
  targets immediately (aligns with "do not trade" / flatten semantics).
- ``REDUCE_ONLY`` / ``PAUSE_NEW_ENTRIES`` do not map 1:1 onto ``allow_long`` / ``allow_short``;
  those modes are enforced primarily by ``RiskEngine`` upstream. Default directional flags stay
  permissive; pass ``allow_long`` / ``allow_short`` if the policy layer needs tighter clamps.
"""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.risk import RiskState, SystemMode
from policy_model.objects import PolicyRiskEnvelope


def risk_state_to_policy_envelope(
    risk: RiskState,
    settings: AppSettings,
    *,
    cooldown_steps_remaining: int = 0,
    max_position_delta_per_step: float = 0.1,
    max_leverage: float = 2.0,
    min_trade_notional: float = 10.0,
    concentration_limit: float = 1.0,
    volatility_limit: float = 1.0,
    daily_loss_limit_remaining: float = 10_000.0,
    allow_long: bool = True,
    allow_short: bool = True,
) -> PolicyRiskEnvelope:
    """Build a `PolicyRiskEnvelope` from the shared `RiskState` and risk limits in settings.

    ``max_abs_position_fraction`` is ``min(1, risk_max_per_symbol_usd / risk_max_total_exposure_usd)``.
    """
    total = max(float(settings.risk_max_total_exposure_usd), 1.0)
    per_sym = max(float(settings.risk_max_per_symbol_usd), 0.0)
    max_abs = min(1.0, per_sym / total) if total > 0 else 1.0

    mode = risk.mode
    kill = mode in (SystemMode.MAINTENANCE, SystemMode.FLATTEN_ALL)

    al, ash = allow_long, allow_short

    return PolicyRiskEnvelope(
        max_abs_position_fraction=max_abs,
        max_position_delta_per_step=max_position_delta_per_step,
        max_leverage=max_leverage,
        min_trade_notional=min_trade_notional,
        cooldown_steps_remaining=cooldown_steps_remaining,
        allow_long=al,
        allow_short=ash,
        kill_switch_active=kill,
        max_drawdown_limit=float(settings.risk_max_drawdown_pct),
        concentration_limit=concentration_limit,
        volatility_limit=volatility_limit,
        daily_loss_limit_remaining=daily_loss_limit_remaining,
    )
