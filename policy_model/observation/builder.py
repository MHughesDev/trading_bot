"""Assemble `PolicyObservation` from forecast packet and state objects."""

from __future__ import annotations

from policy_model.objects import (
    ExecutionState,
    ForecastPacket,
    PolicyObservation,
    PortfolioState,
    RiskState,
)


class PolicyObservationBuilder:
    def build(
        self,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
        history_context: dict | None = None,
    ) -> PolicyObservation:
        ff: list[float] = []
        ff.extend(forecast_packet.q_med)
        ff.extend(forecast_packet.q_low)
        ff.extend(forecast_packet.q_high)
        ff.extend(forecast_packet.interval_width)
        ff.extend(forecast_packet.regime_vector)
        ff.append(
            float(forecast_packet.confidence_score)
            if isinstance(forecast_packet.confidence_score, (int, float))
            else float(sum(forecast_packet.confidence_score) / max(len(forecast_packet.confidence_score), 1))
        )
        ff.extend(forecast_packet.ensemble_variance)
        ff.append(forecast_packet.ood_score)

        pf = [
            portfolio_state.equity,
            portfolio_state.cash,
            portfolio_state.position_fraction,
            portfolio_state.unrealized_pnl,
            portfolio_state.realized_pnl,
            float(portfolio_state.time_in_position),
        ]
        ef = [
            execution_state.mid_price,
            execution_state.spread,
            execution_state.estimated_slippage,
            execution_state.estimated_fee_rate,
            execution_state.volatility_proxy,
        ]
        rf = [
            risk_state.max_abs_position_fraction,
            risk_state.max_position_delta_per_step,
            float(risk_state.cooldown_steps_remaining),
            1.0 if risk_state.kill_switch_active else 0.0,
        ]
        meta = {"history": history_context or {}}
        return PolicyObservation(
            forecast_features=ff,
            portfolio_features=pf,
            execution_features=ef,
            risk_features=rf,
            history_features=None,
            metadata=meta,
        )
