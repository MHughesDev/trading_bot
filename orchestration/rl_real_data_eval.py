"""
RL training/eval on real price paths using the heuristic policy + canonical reward (spec §8).

Full PPO/SAC is backlog; this implements **real-data** step budget and metrics only.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from datetime import UTC, datetime

import numpy as np
import polars as pl

from forecaster_model.config import ForecasterConfig
from forecaster_model.training.real_data_fit import (
    QuantileForecasterArtifact,
    predict_quantile_forecast_packet,
)
from policy_model.objects import ExecutionState, PolicyRiskEnvelope, PortfolioState
from policy_model.policy.heuristic import HeuristicTargetPolicy

logger = logging.getLogger(__name__)


@dataclass
class RLEpisodeMetrics:
    total_return: float
    sharpe_like: float
    max_drawdown: float
    turnover: float
    trade_count: int
    steps: int


def _simple_sharpe(returns: np.ndarray) -> float:
    if len(returns) < 2:
        return 0.0
    mu = float(np.mean(returns))
    sd = float(np.std(returns)) + 1e-12
    return mu / sd * np.sqrt(252.0 * 24.0 * 60.0)  # rough annualization for 1m bars


def run_heuristic_rollout_on_range(
    bars: pl.DataFrame,
    train_range: range,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    *,
    equity_start: float = 100_000.0,
    max_steps: int = 500_000,
    spread_bps: float = 5.0,
    fee_bps: float = 10.0,
) -> RLEpisodeMetrics:
    """
    Walk forward on **train_range** only: each step builds a packet from real OHLC,
    heuristic policy chooses exposure, mark-to-market with log returns.
    """
    o = bars["open"].to_numpy()
    h = bars["high"].to_numpy()
    lo = bars["low"].to_numpy()
    cl = bars["close"].to_numpy()
    vo = bars["volume"].to_numpy()
    policy = HeuristicTargetPolicy(edge_scale=100.0)
    risk = PolicyRiskEnvelope(
        max_abs_position_fraction=1.0,
        max_position_delta_per_step=0.25,
        max_leverage=1.0,
        min_trade_notional=0.0,
        cooldown_steps_remaining=0,
        allow_long=True,
        allow_short=True,
        kill_switch_active=False,
        max_drawdown_limit=0.2,
        concentration_limit=1.0,
        volatility_limit=1.0,
        daily_loss_limit_remaining=1.0,
    )
    L = cfg.history_length
    t_start = train_range.start + L - 1
    t_end = train_range.stop - 2
    equity = equity_start
    peak = equity
    max_dd = 0.0
    turnover = 0.0
    trades = 0
    pos_frac = 0.0
    rets: list[float] = []
    steps = 0
    for t in range(t_start, t_end + 1):
        if t >= len(cl) - 1:
            break
        equity_before = equity
        sl = slice(t - L + 1, t + 1)
        if "timestamp" in bars.columns:
            ts_raw = bars.get_column("timestamp").to_numpy()[t]
            ts = ts_raw if isinstance(ts_raw, datetime) else datetime.fromtimestamp(float(ts_raw), tz=UTC)
        else:
            ts = datetime.now(UTC)
        pkt = predict_quantile_forecast_packet(
            o[sl], h[sl], lo[sl], cl[sl], vo[sl], artifact, cfg, now_ts=ts
        )
        ps = PortfolioState(
            equity=equity,
            cash=equity * (1.0 - abs(pos_frac)),
            position_units=pos_frac * equity / max(cl[t], 1e-12),
            position_notional=abs(pos_frac) * equity,
            position_fraction=pos_frac,
            entry_price=float(cl[t]),
            unrealized_pnl=0.0,
            realized_pnl=0.0,
            current_leverage=1.0,
            time_in_position=steps,
            last_action=None,
            last_trade_timestamp=None,
        )
        mid = float(cl[t])
        slip = spread_bps / 10_000.0 * mid
        es = ExecutionState(
            mid_price=mid,
            spread=spread_bps / 10_000.0 * mid,
            estimated_slippage=slip,
            estimated_fee_rate=fee_bps / 10_000.0,
            available_liquidity_score=1.0,
            latency_proxy=0.01,
            volatility_proxy=float(np.std(np.diff(np.log(cl[max(0, t - 32) : t + 1] + 1e-12)))),
        )
        from policy_model.observation.builder import PolicyObservationBuilder

        obs = PolicyObservationBuilder().build(pkt, ps, es, risk)
        act = policy.select_action(
            obs, forecast_packet=pkt, portfolio_state=ps, risk_state=risk, deterministic=True
        )
        new_pf = float(np.clip(act.target_exposure, -1.0, 1.0))
        turnover += abs(new_pf - pos_frac)
        if abs(new_pf - pos_frac) > 1e-4:
            trades += 1
        pos_frac = new_pf
        r = float(np.log(cl[t + 1] / max(cl[t], 1e-12)))
        fee = abs(new_pf - pos_frac) * equity * (fee_bps / 10_000.0)
        equity = equity * float(np.exp(pos_frac * r)) - fee
        peak = max(peak, equity)
        max_dd = max(max_dd, (peak - equity) / max(peak, 1e-12))
        rets.append((equity - equity_before) / max(equity_before, 1e-12))
        steps += 1
        if steps >= max_steps:
            break
    tot_ret = (equity - equity_start) / equity_start
    return RLEpisodeMetrics(
        total_return=float(tot_ret),
        sharpe_like=_simple_sharpe(np.array(rets, dtype=np.float64)),
        max_drawdown=float(max_dd),
        turnover=float(turnover),
        trade_count=trades,
        steps=steps,
    )
