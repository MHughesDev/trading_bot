"""FB-AUDIT-11: small invariant tests."""

from __future__ import annotations

import numpy as np

from forecaster_model.inference.build_from_ohlc import build_forecast_packet_methodology
from forecaster_model.config import ForecasterConfig
from policy_model.objects import (
    ExecutionState,
    ForecastPacket as PolicyForecastPacket,
    PortfolioState,
    RiskState,
    TargetPosition,
)
from policy_model.risk.gate import RiskGate


def test_quantiles_monotonic_per_horizon() -> None:
    rng = np.random.default_rng(42)
    n = 128
    c = 100 + np.cumsum(rng.normal(0, 0.15, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + 0.01
    lo = np.minimum(o, c) - 0.01
    v = rng.random(n) * 1e6
    cfg = ForecasterConfig(history_length=64, forecast_horizon=8)
    pkt = build_forecast_packet_methodology(o, h, lo, c, v, cfg=cfg, seed=1)
    for i in range(len(pkt.horizons)):
        assert pkt.q_low[i] <= pkt.q_med[i] <= pkt.q_high[i]


def test_risk_gate_max_abs_monotonic() -> None:
    """Larger max_abs_position_fraction never yields smaller |approved| for same target request."""
    fp = PolicyForecastPacket(
        timestamp=__import__("datetime").datetime.now(__import__("datetime").UTC),
        horizons=[1],
        q_low=[0.0],
        q_med=[0.0],
        q_high=[0.0],
        interval_width=[0.01],
        regime_vector=[0.25, 0.25, 0.25, 0.25],
        confidence_score=1.0,
        ensemble_variance=[0.0],
        ood_score=0.0,
    )
    ps = PortfolioState(
        equity=100_000.0,
        cash=100_000.0,
        position_units=0.0,
        position_notional=0.0,
        position_fraction=0.0,
        entry_price=None,
        unrealized_pnl=0.0,
        realized_pnl=0.0,
        current_leverage=1.0,
        time_in_position=0,
        last_action=None,
        last_trade_timestamp=None,
    )
    es = ExecutionState(
        mid_price=50_000.0,
        spread=1.0,
        estimated_slippage=0.1,
        estimated_fee_rate=0.001,
        available_liquidity_score=1.0,
        latency_proxy=0.01,
        volatility_proxy=0.02,
    )
    gate = RiskGate()
    tgt = TargetPosition(
        target_fraction=0.9,
        target_units=None,
        target_notional=None,
        target_leverage=1.0,
        reason_codes=[],
    )
    base_rs = RiskState(
        max_abs_position_fraction=0.3,
        max_position_delta_per_step=1.0,
        max_leverage=2.0,
        min_trade_notional=1.0,
        cooldown_steps_remaining=0,
        allow_long=True,
        allow_short=True,
        kill_switch_active=False,
        max_drawdown_limit=1.0,
        concentration_limit=1.0,
        volatility_limit=1.0,
        daily_loss_limit_remaining=1.0,
    )
    a_small = gate.evaluate(tgt, fp, ps, es, base_rs).approved_target_fraction
    rs_looser = RiskState(
        max_abs_position_fraction=0.6,
        max_position_delta_per_step=1.0,
        max_leverage=2.0,
        min_trade_notional=1.0,
        cooldown_steps_remaining=0,
        allow_long=True,
        allow_short=True,
        kill_switch_active=False,
        max_drawdown_limit=1.0,
        concentration_limit=1.0,
        volatility_limit=1.0,
        daily_loss_limit_remaining=1.0,
    )
    a_large = gate.evaluate(tgt, fp, ps, es, rs_looser).approved_target_fraction
    assert abs(a_large) >= abs(a_small)
