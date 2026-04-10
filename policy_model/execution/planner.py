"""Execution plan from approved target vs current position (human spec §14)."""

from __future__ import annotations

from policy_model.objects import ApprovedTarget, ExecutionPlan, ExecutionState, PortfolioState


class ExecutionPlanner:
    def __init__(self, min_effective_delta: float = 1e-4) -> None:
        self._min = min_effective_delta

    def plan(
        self,
        approved_target: ApprovedTarget,
        portfolio_state: PortfolioState,
        execution_state: ExecutionState,
    ) -> ExecutionPlan:
        if not approved_target.approved:
            return ExecutionPlan(
                required_delta_fraction=0.0,
                required_delta_notional=0.0,
                execution_mode="none",
                max_child_order_size=None,
                urgency=0.0,
                skip_execution=True,
                skip_reasons=approved_target.rejection_reasons or ["not_approved"],
            )

        cur = portfolio_state.position_fraction
        tgt = approved_target.approved_target_fraction
        delta = tgt - cur
        if abs(delta) < self._min:
            return ExecutionPlan(
                required_delta_fraction=delta,
                required_delta_notional=abs(delta) * portfolio_state.equity,
                execution_mode="hold",
                max_child_order_size=None,
                urgency=0.0,
                skip_execution=True,
                skip_reasons=["below_min_delta"],
            )

        return ExecutionPlan(
            required_delta_fraction=delta,
            required_delta_notional=abs(delta) * portfolio_state.equity,
            execution_mode="rebalance",
            max_child_order_size=None,
            urgency=min(1.0, abs(delta) / max(execution_state.volatility_proxy, 1e-6)),
            skip_execution=False,
            skip_reasons=[],
        )
