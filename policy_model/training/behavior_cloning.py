"""Conservative offline RL: behavior cloning stub (FB-PL-PG8)."""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from policy_model.objects import PolicyObservation
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy


@dataclass
class BCDataset:
    observations: list[PolicyObservation]
    expert_actions: list[float]


def behavior_cloning_loss(policy: MultiBranchMLPPolicy, batch: BCDataset) -> float:
    """MSE between policy target_exposure and expert scalar actions."""
    if not batch.observations:
        return 0.0
    errs = []
    for obs, expert_a in zip(batch.observations, batch.expert_actions, strict=True):
        a = policy.select_action(obs, deterministic=True).target_exposure
        errs.append((a - expert_a) ** 2)
    return float(np.mean(errs))
