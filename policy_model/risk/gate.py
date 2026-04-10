"""Deterministic risk gate for policy targets (human spec §13)."""

from __future__ import annotations

from policy_model.objects import (
    ApprovedTarget,
    ExecutionState,
    ForecastPacket,
    PortfolioState,
    RiskState,
    TargetPosition,
)


class RiskGate:
    def evaluate(
        self,
        target: TargetPosition,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
    ) -> ApprovedTarget:
        reasons: list[str] = []
        clamps: list[str] = []

        if risk_state.kill_switch_active:
            return ApprovedTarget(
                approved=False,
                approved_target_fraction=0.0,
                rejection_reasons=["kill_switch"],
                clamp_reasons=[],
                risk_diagnostics={"kill_switch": True},
            )

        conf = (
            float(forecast_packet.confidence_score)
            if isinstance(forecast_packet.confidence_score, (int, float))
            else float(
                sum(forecast_packet.confidence_score) / max(len(forecast_packet.confidence_score), 1)
            )
        )
        if conf < 1e-6 and max(forecast_packet.interval_width) > 1e-3:
            reasons.append("low_confidence_wide_interval")

        frac = target.target_fraction
        if not risk_state.allow_short and frac < 0:
            clamps.append("no_short")
            frac = max(0.0, frac)
        if not risk_state.allow_long and frac > 0:
            clamps.append("no_long")
            frac = min(0.0, frac)

        m = risk_state.max_abs_position_fraction
        if abs(frac) > m:
            clamps.append("max_abs_position")
            frac = max(-m, min(m, frac))

        delta = abs(frac - portfolio_state.position_fraction)
        if delta > risk_state.max_position_delta_per_step:
            clamps.append("max_delta")
            step = risk_state.max_position_delta_per_step
            direction = 1.0 if frac > portfolio_state.position_fraction else -1.0
            frac = portfolio_state.position_fraction + direction * step

        # Approve clamped targets; reject only when nothing is salvageable (stub: always approve if not killed)
        approved = True
        return ApprovedTarget(
            approved=approved,
            approved_target_fraction=frac,
            rejection_reasons=reasons if not approved else [],
            clamp_reasons=clamps,
            risk_diagnostics={"conf": conf},
        )
