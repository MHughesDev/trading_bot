"""
RL training/eval on real price paths using the canonical reward (spec §8).

Two rollout drivers share one walk-forward loop:
- ``run_heuristic_rollout_on_range`` — the heuristic target policy (baseline).
- ``run_policy_rollout_on_range`` — a trained ``PolicyNetwork`` (deterministic eval).

``train_actor_critic_on_range`` trains a ``PolicyNetwork`` on a real-bar range with the
advantage-weighted actor-critic update (torch-free) and returns the deterministic eval
metrics, optionally persisting the policy ``.npz`` for the serving path.
"""

from __future__ import annotations

import logging
from collections.abc import Callable
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

import numpy as np
import polars as pl

from forecaster_model.config import ForecasterConfig
from forecaster_model.training.real_data_fit import (
    QuantileForecasterArtifact,
    predict_quantile_forecast_packet,
)
from policy_model.objects import (
    ExecutionState,
    PolicyObservation,
    PolicyRiskEnvelope,
    PortfolioState,
)
from policy_model.observation.builder import PolicyObservationBuilder
from policy_model.policy.heuristic import HeuristicTargetPolicy

logger = logging.getLogger(__name__)

# select_fn(obs, packet, portfolio_state, risk) -> target exposure in [-1, 1]
SelectFn = Callable[[PolicyObservation, object, PortfolioState, PolicyRiskEnvelope], float]


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


def _default_risk_envelope() -> PolicyRiskEnvelope:
    return PolicyRiskEnvelope(
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


def _ts_at(bars: pl.DataFrame, t: int) -> datetime:
    if "timestamp" in bars.columns:
        ts_raw = bars.get_column("timestamp").to_numpy()[t]
        if isinstance(ts_raw, datetime):
            return ts_raw
        return datetime.fromtimestamp(float(ts_raw), tz=UTC)
    return datetime.now(UTC)


def _build_obs(
    arrays: tuple[np.ndarray, ...],
    t: int,
    L: int,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    *,
    ts: datetime,
    equity: float,
    pos_frac: float,
    steps: int,
    spread_bps: float,
    fee_bps: float,
    risk: PolicyRiskEnvelope,
) -> tuple[PolicyObservation, object, PortfolioState]:
    o, h, lo, cl, vo = arrays
    sl = slice(t - L + 1, t + 1)
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
    obs = PolicyObservationBuilder().build(pkt, ps, es, risk)
    return obs, pkt, ps


def _rollout_on_range(
    bars: pl.DataFrame,
    train_range: range,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    select_fn: SelectFn,
    *,
    equity_start: float,
    max_steps: int,
    spread_bps: float,
    fee_bps: float,
) -> RLEpisodeMetrics:
    """Walk forward on ``train_range``: build a packet per step, pick exposure, mark to market."""
    arrays = (
        bars["open"].to_numpy(),
        bars["high"].to_numpy(),
        bars["low"].to_numpy(),
        bars["close"].to_numpy(),
        bars["volume"].to_numpy(),
    )
    cl = arrays[3]
    risk = _default_risk_envelope()
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
        obs, pkt, ps = _build_obs(
            arrays, t, L, artifact, cfg, ts=_ts_at(bars, t), equity=equity,
            pos_frac=pos_frac, steps=steps, spread_bps=spread_bps, fee_bps=fee_bps, risk=risk,
        )
        new_pf = float(np.clip(select_fn(obs, pkt, ps, risk), -1.0, 1.0))
        delta = new_pf - pos_frac  # compute trade delta BEFORE updating exposure
        turnover += abs(delta)
        if abs(delta) > 1e-4:
            trades += 1
        fee = abs(delta) * equity * (fee_bps / 10_000.0)
        r = float(np.log(cl[t + 1] / max(cl[t], 1e-12)))
        equity = equity * float(np.exp(new_pf * r)) - fee
        pos_frac = new_pf
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
    """Baseline rollout driven by the heuristic target policy."""
    policy = HeuristicTargetPolicy(edge_scale=100.0)

    def _select(obs: PolicyObservation, pkt: object, ps: PortfolioState, risk: PolicyRiskEnvelope) -> float:
        act = policy.select_action(
            obs, forecast_packet=pkt, portfolio_state=ps, risk_state=risk, deterministic=True
        )
        return float(act.target_exposure)

    return _rollout_on_range(
        bars, train_range, artifact, cfg, _select,
        equity_start=equity_start, max_steps=max_steps, spread_bps=spread_bps, fee_bps=fee_bps,
    )


def run_policy_rollout_on_range(
    bars: pl.DataFrame,
    train_range: range,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    *,
    policy: object,
    equity_start: float = 100_000.0,
    max_steps: int = 500_000,
    spread_bps: float = 5.0,
    fee_bps: float = 10.0,
) -> RLEpisodeMetrics:
    """Deterministic evaluation rollout driven by a trained policy (``select_action(obs)``)."""

    def _select(obs: PolicyObservation, _pkt: object, _ps: PortfolioState, _risk: PolicyRiskEnvelope) -> float:
        return float(policy.select_action(obs, deterministic=True).target_exposure)

    return _rollout_on_range(
        bars, train_range, artifact, cfg, _select,
        equity_start=equity_start, max_steps=max_steps, spread_bps=spread_bps, fee_bps=fee_bps,
    )


def train_actor_critic_on_range(
    bars: pl.DataFrame,
    train_range: range,
    artifact: QuantileForecasterArtifact,
    cfg: ForecasterConfig,
    *,
    equity_start: float = 100_000.0,
    max_steps: int = 500_000,
    spread_bps: float = 5.0,
    fee_bps: float = 10.0,
    epochs: int = 3,
    seed: int = 0,
    update_every: int = 16,
    lam_turn: float = 0.001,
    save_policy_path: str | Path | None = None,
) -> RLEpisodeMetrics:
    """Train a ``PolicyNetwork`` (advantage-weighted actor-critic) on real bars, then evaluate.

    Collects transitions by rolling the (exploring) policy across ``train_range`` each epoch,
    rewards each step with ``exposure·logret − fee − λ·turnover``, and applies the actor-critic
    update every ``update_every`` transitions. Returns the deterministic eval metrics; persists
    the policy ``.npz`` when ``save_policy_path`` is given.
    """
    from policy_model.objects import PolicyAction
    from policy_model.policy.policy_network import PolicyNetwork
    from policy_model.training.buffer import ReplayBuffer, Transition

    arrays = (
        bars["open"].to_numpy(),
        bars["high"].to_numpy(),
        bars["low"].to_numpy(),
        bars["close"].to_numpy(),
        bars["volume"].to_numpy(),
    )
    cl = arrays[3]
    risk = _default_risk_envelope()
    L = cfg.history_length
    t_start = train_range.start + L - 1
    t_end = train_range.stop - 2

    policy = PolicyNetwork(seed=seed)
    buf = ReplayBuffer(capacity=4096)

    for _epoch in range(max(1, epochs)):
        equity = equity_start
        pos_frac = 0.0
        steps = 0
        prev: tuple[PolicyObservation, PolicyAction, float] | None = None
        for t in range(t_start, t_end + 1):
            if t >= len(cl) - 1:
                break
            obs, _pkt, _ps = _build_obs(
                arrays, t, L, artifact, cfg, ts=_ts_at(bars, t), equity=equity,
                pos_frac=pos_frac, steps=steps, spread_bps=spread_bps, fee_bps=fee_bps, risk=risk,
            )
            if prev is not None:
                buf.push(Transition(prev[0], prev[1], prev[2], obs, False, {}))
                if len(buf) % update_every == 0:
                    policy.update(buf)
            act = policy.select_action(obs, deterministic=False)
            new_pf = float(np.clip(act.target_exposure, -1.0, 1.0))
            delta = new_pf - pos_frac
            r = float(np.log(cl[t + 1] / max(cl[t], 1e-12)))
            fee_frac = abs(delta) * (fee_bps / 10_000.0)
            reward = new_pf * r - fee_frac - lam_turn * abs(delta)
            equity = equity * float(np.exp(new_pf * r)) - abs(delta) * equity * (fee_bps / 10_000.0)
            prev = (obs, PolicyAction(target_exposure=new_pf), reward)
            pos_frac = new_pf
            steps += 1
            if steps >= max_steps:
                break
        if prev is not None:
            buf.push(Transition(prev[0], prev[1], prev[2], prev[0], True, {}))
        policy.update(buf)

    if save_policy_path is not None:
        policy.save(str(save_policy_path))

    return run_policy_rollout_on_range(
        bars, train_range, artifact, cfg, policy=policy,
        equity_start=equity_start, max_steps=max_steps, spread_bps=spread_bps, fee_bps=fee_bps,
    )
