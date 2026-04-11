"""Action projection — human policy spec §12 `ActionProjector`."""

from __future__ import annotations

from policy_model.objects import PolicyAction, PortfolioState, RiskState, TargetPosition


class ActionProjector:
    def project(
        self,
        action: PolicyAction,
        portfolio_state: PortfolioState,
        risk_state: RiskState,
    ) -> TargetPosition:
        tf = action.target_exposure * risk_state.max_abs_position_fraction
        if not risk_state.allow_short:
            tf = max(0.0, tf)
        if not risk_state.allow_long:
            tf = min(0.0, tf)
        return TargetPosition(
            target_fraction=tf,
            target_units=None,
            target_notional=None,
            target_leverage=portfolio_state.current_leverage,
            reason_codes=["action_projection"],
        )
