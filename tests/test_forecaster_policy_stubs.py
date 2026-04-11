"""Forecaster stub + policy pipeline smoke tests."""

from __future__ import annotations

from datetime import UTC, datetime

import numpy as np

from forecaster_model.inference.stub import build_forecast_packet_stub
from policy_model.objects import ExecutionState, PortfolioState, RiskState
from policy_model.system import PolicySystem


def _synth_bars(n: int = 128) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    rng = np.random.default_rng(42)
    c = 100 + np.cumsum(rng.normal(0, 0.1, size=n))
    o = np.roll(c, 1)
    o[0] = c[0]
    h = np.maximum(o, c) + rng.random(n) * 0.05
    lo = np.minimum(o, c) - rng.random(n) * 0.05
    v = rng.random(n) * 1e6 + 1
    return o, h, lo, c, v


def test_forecast_packet_stub_shape() -> None:
    o, h, lo, c, v = _synth_bars()
    pkt = build_forecast_packet_stub(o, h, lo, c, v, now=datetime.now(UTC))
    assert len(pkt.q_med) == len(pkt.horizons)
    assert len(pkt.regime_vector) == 4


def test_policy_system_runs() -> None:
    o, h, lo, c, v = _synth_bars()
    pkt = build_forecast_packet_stub(o, h, lo, c, v)
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
        mid_price=float(c[-1]),
        spread=1.0,
        estimated_slippage=0.5,
        estimated_fee_rate=0.0005,
        available_liquidity_score=1.0,
        latency_proxy=0.01,
        volatility_proxy=0.02,
    )
    re = RiskState(
        max_abs_position_fraction=0.25,
        max_position_delta_per_step=0.1,
        max_leverage=2.0,
        min_trade_notional=10.0,
        cooldown_steps_remaining=0,
        allow_long=True,
        allow_short=True,
        kill_switch_active=False,
        max_drawdown_limit=0.2,
        concentration_limit=1.0,
        volatility_limit=1.0,
        daily_loss_limit_remaining=10_000.0,
    )
    sys = PolicySystem()
    out = sys.decide(pkt, ps, es, re)
    assert "execution_plan" in out
    assert out["execution_plan"].skip_execution in (True, False)
