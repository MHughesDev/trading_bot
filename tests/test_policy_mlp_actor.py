"""Multi-branch MLP policy (policy human spec §10)."""

from __future__ import annotations

from policy_model.objects import PolicyObservation
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy


def test_mlp_policy_bounded_action() -> None:
    pol = MultiBranchMLPPolicy(seed=1)
    obs = PolicyObservation(
        forecast_features=[0.1] * 40,
        portfolio_features=[1.0] * 10,
        execution_features=[100.0] * 10,
        risk_features=[0.2] * 10,
        history_features=None,
        metadata={},
    )
    a = pol.select_action(obs, deterministic=True)
    assert -1.0 <= a.target_exposure <= 1.0
