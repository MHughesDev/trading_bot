"""FB-PL-PG5: planner respects min notional style constraints."""

from policy_model.objects import ApprovedTarget, ExecutionState, PortfolioState
from policy_model.execution.planner import ExecutionPlanner


def test_skip_when_delta_notional_below_min():
    planner = ExecutionPlanner(min_effective_delta=1e-6)
    ps = PortfolioState(
        equity=10_000.0,
        cash=10_000.0,
        position_units=0.0,
        position_notional=0.0,
        position_fraction=0.0,
        entry_price=None,
        unrealized_pnl=0.0,
        realized_pnl=0.0,
        current_leverage=1.0,
        time_in_position=0,
        last_action=None,
        last_trade_timestamp=None,
    )
    es = ExecutionState(
        mid_price=50_000.0,
        spread=1.0,
        estimated_slippage=0.5,
        estimated_fee_rate=0.001,
        available_liquidity_score=1.0,
        latency_proxy=0.01,
        volatility_proxy=0.02,
    )
    # tiny delta -> below min for any reasonable min_notional; we model as hold/skip
    ap = ApprovedTarget(
        approved=True,
        approved_target_fraction=1e-8,
        rejection_reasons=[],
        clamp_reasons=[],
        risk_diagnostics={},
    )
    plan = planner.plan(ap, ps, es)
    assert plan.skip_execution
