"""
PolicyNetwork + PolicyAlgorithm facade (human policy spec §10.4, §19.3).

Multi-branch encoder is in `mlp_actor.MultiBranchMLPPolicy`; this module exposes
`select_action` / `update` matching the canonical `PolicyAlgorithm` interface.
"""

from __future__ import annotations

from typing import Any

from policy_model.objects import PolicyAction, PolicyObservation
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from policy_model.training.protocol import RLPolicyAlgorithm


class PolicyNetwork(RLPolicyAlgorithm):
    """Wraps `MultiBranchMLPPolicy` with stub `update` for future RL training."""

    def __init__(self, seed: int = 0) -> None:
        self._actor = MultiBranchMLPPolicy(seed=seed)

    def select_action(self, obs: PolicyObservation, deterministic: bool = True) -> PolicyAction:
        return self._actor.select_action(obs, deterministic=deterministic)

    def update(self, batch: Any) -> dict[str, float]:
        _ = batch
        return {"loss": 0.0}

    def save(self, path: str) -> None:
        _ = path

    def load(self, path: str) -> None:
        _ = path
