"""Heuristic policy: edge / uncertainty → target exposure (baseline actor before RL training)."""

from __future__ import annotations

from app.contracts.forecast_packet import ForecastPacket
from policy_model.objects import PolicyAction, PolicyObservation, PortfolioState, RiskState


class HeuristicTargetPolicy:
    """Maps forecast median path vs interval width to [-1, 1] target exposure (spec §11 canonical action)."""

    def __init__(self, edge_scale: float = 100.0) -> None:
        self._edge_scale = edge_scale

    def select_action(
        self,
        obs: PolicyObservation,
        *,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        risk_state: RiskState,
        deterministic: bool = True,
    ) -> PolicyAction:
        _ = obs
        _ = deterministic
        med = forecast_packet.q_med
        width = forecast_packet.interval_width
        edge = sum(med) / max(len(med), 1)
        unc = sum(width) / max(len(width), 1) + 1e-9
        raw = self._edge_scale * edge / unc
        churn = 0.2 * portfolio_state.position_fraction
        a = raw - churn
        m = risk_state.max_abs_position_fraction
        a = max(-1.0, min(1.0, a / max(m, 1e-9)))
        return PolicyAction(
            target_exposure=a,
            action_diagnostics={"edge": edge, "unc": unc},
        )
