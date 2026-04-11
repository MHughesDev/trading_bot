"""
Offline RL environment using **precomputed** `ForecastPacket` rows (FB-PL-P0-03).

Skips `DecisionPipeline`; observation vectors match live/replay via `PolicyObservationBuilder`.
Use when training from historical forecaster outputs without re-running inference.
"""

from __future__ import annotations

import math
from typing import Any, Sequence

import numpy as np

from app.config.settings import AppSettings, load_settings
from app.contracts.forecast_packet import ForecastPacket
from app.contracts.risk import RiskState
from policy_model.bridge import policy_envelope_from_app_settings
from policy_model.env.reward import one_step_reward
from policy_model.objects import (
    ExecutionState,
    PolicyAction,
    PolicyObservation,
    PortfolioState,
)
from policy_model.observation.builder import PolicyObservationBuilder


def _minimal_portfolio(equity: float, position_fraction: float) -> PortfolioState:
    return PortfolioState(
        equity=equity,
        cash=equity * (1.0 - abs(position_fraction)),
        position_units=0.0,
        position_notional=equity * abs(position_fraction),
        position_fraction=position_fraction,
        entry_price=None,
        unrealized_pnl=0.0,
        realized_pnl=0.0,
        current_leverage=1.0,
        time_in_position=0,
        last_action=None,
        last_trade_timestamp=None,
    )


def _minimal_execution(mid: float, spread_bps: float, settings: AppSettings) -> ExecutionState:
    slip = settings.backtesting_slippage_bps / 10_000.0 * mid
    fee = settings.backtesting_fee_bps / 10_000.0
    return ExecutionState(
        mid_price=mid,
        spread=spread_bps / 10_000.0 * mid,
        estimated_slippage=slip,
        estimated_fee_rate=fee,
        available_liquidity_score=1.0,
        latency_proxy=0.01,
        volatility_proxy=0.02,
    )


class OfflineForecastPacketEnvironment:
    """
    `reset` / `step` over aligned `ForecastPacket` and mid-price series.

    `packets` and `mids` must have the same length (>= 2).
    """

    def __init__(
        self,
        packets: Sequence[ForecastPacket],
        mids: np.ndarray,
        *,
        spread_bps: float = 5.0,
        settings: AppSettings | None = None,
        initial_equity: float = 100_000.0,
    ) -> None:
        self._packets = list(packets)
        self._mids = np.asarray(mids, dtype=np.float64).ravel()
        if len(self._packets) != len(self._mids):
            raise ValueError("packets and mids must have the same length")
        if len(self._packets) < 2:
            raise ValueError("need at least 2 timesteps")
        self._spread_bps = float(spread_bps)
        self._settings = settings or load_settings()
        self._initial_equity = float(initial_equity)
        self._idx = 0
        self._position_fraction = 0.0
        self._risk_app = RiskState()
        self._builder = PolicyObservationBuilder()
        self._done = False

    def reset(self) -> PolicyObservation:
        self._idx = 0
        self._position_fraction = 0.0
        self._risk_app = RiskState()
        self._done = False
        return self._build_obs_at_index()

    def _build_obs_at_index(self) -> PolicyObservation:
        mid = float(self._mids[self._idx])
        pkt = self._packets[self._idx]
        env_risk = policy_envelope_from_app_settings(self._settings, self._risk_app)
        ps = _minimal_portfolio(self._initial_equity, self._position_fraction)
        es = _minimal_execution(mid, self._spread_bps, self._settings)
        return self._builder.build(
            forecast_packet=pkt,
            portfolio_state=ps,
            execution_state=es,
            risk_state=env_risk,
        )

    def step(self, action: PolicyAction) -> tuple[PolicyObservation, float, bool, dict[str, Any]]:
        if self._done:
            obs = self._terminal_observation()
            return obs, 0.0, True, {"reason": "already_done"}

        if self._idx >= len(self._mids) - 1:
            self._done = True
            obs = self._terminal_observation()
            return obs, 0.0, True, {"reason": "episode_end"}

        c0 = float(self._mids[self._idx])
        c1 = float(self._mids[self._idx + 1])
        r_mkt = math.log(max(c1, 1e-12) / max(c0, 1e-12))

        prev_pos = self._position_fraction
        new_pos = float(np.clip(action.target_exposure * 0.25, -1.0, 1.0))
        turnover = abs(new_pos - prev_pos)
        self._position_fraction = new_pos

        delta_log_equity = prev_pos * r_mkt
        fee_rt = (self._settings.backtesting_fee_bps + self._settings.backtesting_slippage_bps) / 10_000.0
        cost = fee_rt * turnover

        rew = one_step_reward(
            delta_log_equity,
            turnover,
            cost,
            lam_turn=0.01,
            lam_cost=1.0,
        )

        self._idx += 1
        done = self._idx >= len(self._mids) - 1
        self._done = done
        obs = self._build_obs_at_index() if not done else self._terminal_observation()
        return obs, rew, done, {"mid": float(self._mids[self._idx]), "turnover": turnover}

    def _terminal_observation(self) -> PolicyObservation:
        idx = min(self._idx, len(self._packets) - 1)
        mid = float(self._mids[idx])
        pkt = self._packets[idx]
        return self._builder.build(
            forecast_packet=pkt,
            portfolio_state=_minimal_portfolio(self._initial_equity, self._position_fraction),
            execution_state=_minimal_execution(mid, self._spread_bps, self._settings),
            risk_state=policy_envelope_from_app_settings(self._settings, self._risk_app),
        )
