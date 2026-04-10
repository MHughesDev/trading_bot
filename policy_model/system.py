"""Top-level policy pipeline (human spec §28)."""

from __future__ import annotations

from app.contracts.forecast_packet import ForecastPacket
from policy_model.execution.planner import ExecutionPlanner
from policy_model.objects import ExecutionState, PortfolioState, RiskState
from policy_model.observation.builder import PolicyObservationBuilder
from policy_model.policy.action_projection import ActionProjector
from policy_model.policy.heuristic import HeuristicTargetPolicy
from policy_model.risk.gate import RiskGate
from policy_model.training.protocol import RLPolicyAlgorithm


class PolicySystem:
    """Human policy spec §28 — `policy_algorithm` implements `select_action` / `update`."""

    def __init__(
        self,
        observation_builder: PolicyObservationBuilder | None = None,
        policy_algorithm: RLPolicyAlgorithm | None = None,
        action_projector: ActionProjector | None = None,
        risk_gate: RiskGate | None = None,
        planner: ExecutionPlanner | None = None,
    ) -> None:
        self.observation_builder = observation_builder or PolicyObservationBuilder()
        self.policy_algorithm = policy_algorithm or HeuristicTargetPolicy()
        self.action_projector = action_projector or ActionProjector()
        self.risk_gate = risk_gate or RiskGate()
        self.planner = planner or ExecutionPlanner()

    def decide(
        self,
        forecast_packet: ForecastPacket,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
        risk_state: RiskState,
        *,
        history_context: dict | None = None,
        deterministic: bool = True,
    ) -> dict:
        obs = self.observation_builder.build(
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
            risk_state=risk_state,
            history_context=history_context,
        )
        pa = self.policy_algorithm
        if isinstance(pa, HeuristicTargetPolicy):
            action = pa.select_action(
                obs,
                forecast_packet=forecast_packet,
                portfolio_state=portfolio_state,
                risk_state=risk_state,
                deterministic=deterministic,
            )
        else:
            action = pa.select_action(obs, deterministic=deterministic)
        target = self.action_projector.project(action, portfolio_state, risk_state)
        approved = self.risk_gate.evaluate(
            target=target,
            forecast_packet=forecast_packet,
            portfolio_state=portfolio_state,
            execution_state=execution_state,
            risk_state=risk_state,
        )
        plan = self.planner.plan(approved, portfolio_state, execution_state)
        return {
            "observation": obs,
            "action": action,
            "target": target,
            "approved_target": approved,
            "execution_plan": plan,
        }
