"""Conservative offline RL: behavior cloning (FB-PL-PG8).

Real gradient-descent cloning of expert target_exposure actions into the NumPy MLP actor
via :meth:`MultiBranchMLPPolicy.backward` / :meth:`apply_grads` — no torch required.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

from legacy.decision_pipeline.policy_model.objects import PolicyObservation
from legacy.decision_pipeline.policy_model.policy.mlp_actor import MultiBranchMLPPolicy


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


def _zero_grads_like(policy: MultiBranchMLPPolicy) -> dict[str, np.ndarray]:
    return {
        "w_f": np.zeros_like(policy._w_f),
        "w_p": np.zeros_like(policy._w_p),
        "w_e": np.zeros_like(policy._w_e),
        "w_r": np.zeros_like(policy._w_r),
        "w_out": np.zeros_like(policy._w_out),
    }


def train_behavior_cloning(
    policy: MultiBranchMLPPolicy,
    batch: BCDataset,
    *,
    epochs: int = 50,
    lr: float = 0.05,
) -> dict[str, float]:
    """Full-batch gradient descent minimizing MSE to expert actions; returns loss trajectory.

    Returns ``{"loss", "first_loss", "n", "epochs"}``; ``loss`` is the final-epoch MSE.
    """
    n = len(batch.observations)
    if n == 0:
        return {"loss": 0.0, "first_loss": 0.0, "n": 0.0, "epochs": 0.0}
    first_loss = float("nan")
    loss = float("nan")
    for ep in range(max(1, epochs)):
        grads = _zero_grads_like(policy)
        sq = 0.0
        for obs, expert_a in zip(batch.observations, batch.expert_actions, strict=True):
            a, cache = policy.forward_with_cache(obs)
            sq += (a - expert_a) ** 2
            d_loss_d_a = 2.0 * (a - expert_a) / n
            g = policy.backward(cache, d_loss_d_a)
            for k in grads:
                grads[k] += g[k]
        policy.apply_grads(grads, lr)
        loss = sq / n
        if ep == 0:
            first_loss = loss
    return {"loss": loss, "first_loss": first_loss, "n": float(n), "epochs": float(max(1, epochs))}
