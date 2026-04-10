"""Top-level policy pipeline (human spec §28)."""

from __future__ import annotations

from app.contracts.forecast_packet import ForecastPacket
from policy_model.execution.planner import ExecutionPlanner
from policy_model.objects import (
    ExecutionState,
    PolicyRiskEnvelope,
    PortfolioState,
)
from policy_model.observation.builder import PolicyObservationBuilder
from policy_model.policy.heuristic import ActionProjector, HeuristicTargetPolicy
from policy_model.risk.gate import PolicyRiskGate


class PolicySystem:
    def __init__(
        self,
        observation_builder: PolicyObservationBuilder | None = None,
        policy: HeuristicTargetPolicy | None = None,
        projector: ActionProjector | None = None,
        risk_gate: PolicyRiskGate | None = None,
        planner: ExecutionPlanner | None = None,
    ) -> None:
        self.observation_builder = observation_builder or PolicyObservationBuilder()
        self.policy = policy or HeuristicTargetPolicy()
        self.projector = projector or ActionProjector()
        self.risk_gate = risk_gate or PolicyRiskGate()
        self.planner = planner or ExecutionPlanner()

    def decide(
        self,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_envelope: PolicyRiskEnvelope,
        *,
        history_context: dict | None = None,
        deterministic: bool = True,
    ) -> dict:
        obs = self.observation_builder.build(
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
            risk_state=risk_envelope,
            history_context=history_context,
        )
        action = self.policy.select_action(
            obs,
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            risk_state=risk_envelope,
            deterministic=deterministic,
        )
        target = self.projector.project(action, portfolio_state, risk_envelope)
        approved = self.risk_gate.evaluate(
            target=target,
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
            risk_state=risk_envelope,
        )
        plan = self.planner.plan(approved, portfolio_state, execution_state)
        return {
            "observation": obs,
            "action": action,
            "target": target,
            "approved_target": approved,
            "execution_plan": plan,
        }
