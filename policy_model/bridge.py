"""Map `AppSettings` + app `RiskState` → policy-layer `RiskState` (spec §8.4) for `PolicySystem`."""

from __future__ import annotations

from app.config.settings import AppSettings
from app.contracts.risk import RiskState, SystemMode
from policy_model.objects import RiskState as PolicyRiskState


def policy_envelope_from_app_settings(
    settings: AppSettings,
    risk: RiskState,
    *,
    max_position_delta_per_step: float = 0.25,
    min_trade_notional: float = 10.0,
    cooldown_steps_remaining: int = 0,
) -> PolicyRiskState:
    """
    Derive policy-layer limits from production risk settings and runtime `RiskState`.

    This is a **bridge**, not a duplicate of `RiskEngine` rules: execution still uses
    `RiskEngine`; this supplies the policy spec's envelope for `PolicySystem` / diagnostics.
    """
    total = max(settings.risk_max_total_exposure_usd, 1e-9)
    per_sym = settings.risk_max_per_symbol_usd
    max_frac = min(1.0, per_sym / total)

    kill = risk.mode == SystemMode.MAINTENANCE
    if risk.mode == SystemMode.PAUSE_NEW_ENTRIES:
        allow_long = False
        allow_short = False
    elif risk.mode == SystemMode.MAINTENANCE:
        allow_long = False
        allow_short = False
    else:
        # REDUCE_ONLY / FLATTEN_ALL / RUNNING: envelope allows both; mode semantics enforced in RiskEngine
        allow_long = True
        allow_short = True

    return PolicyRiskState(
        max_abs_position_fraction=max_frac,
        max_position_delta_per_step=max_position_delta_per_step,
        max_leverage=1.0,
        min_trade_notional=min_trade_notional,
        cooldown_steps_remaining=cooldown_steps_remaining,
        allow_long=allow_long,
        allow_short=allow_short,
        kill_switch_active=kill,
        max_drawdown_limit=float(settings.risk_max_drawdown_pct),
        concentration_limit=1.0,
        volatility_limit=1.0,
        daily_loss_limit_remaining=float(settings.risk_max_total_exposure_usd),
    )
