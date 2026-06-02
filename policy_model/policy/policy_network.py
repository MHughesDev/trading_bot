"""
PolicyNetwork + PolicyAlgorithm facade (human policy spec §10.4, §19.3).

Multi-branch encoder is in `mlp_actor.MultiBranchMLPPolicy`; this module exposes
`select_action` / `update` matching the canonical `PolicyAlgorithm` interface. `update`
dispatches to the real NumPy trainers: a `BCDataset` runs one behavior-cloning step, and a
`ReplayBuffer` / list of `Transition` runs an advantage-weighted actor-critic step.
"""

from __future__ import annotations

from typing import Any

from policy_model.objects import PolicyAction, PolicyObservation
from policy_model.policy.critic import ValueCritic
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from policy_model.training.actor_critic import ActorCriticTrainer
from policy_model.training.protocol import RLPolicyAlgorithm


class PolicyNetwork(RLPolicyAlgorithm):
    """Wraps `MultiBranchMLPPolicy` with a real RL `update` (behavior cloning / actor-critic)."""

    def __init__(self, seed: int = 0) -> None:
        self._actor = MultiBranchMLPPolicy(seed=seed)
        self._critic = ValueCritic(seed=seed)
        self._trainer = ActorCriticTrainer(self._actor, critic=self._critic)

    def select_action(self, obs: PolicyObservation, deterministic: bool = True) -> PolicyAction:
        return self._actor.select_action(obs, deterministic=deterministic)

    def update(self, batch: Any) -> dict[str, float]:
        # Late imports avoid a module import cycle (training packages import the actor).
        from policy_model.training.behavior_cloning import BCDataset, train_behavior_cloning
        from policy_model.training.buffer import ReplayBuffer, Transition

        if isinstance(batch, BCDataset):
            return train_behavior_cloning(self._actor, batch, epochs=1)
        if isinstance(batch, ReplayBuffer):
            return self._trainer.update_from_buffer(batch, batch_size=min(64, max(1, len(batch))))
        if isinstance(batch, list) and batch and isinstance(batch[0], Transition):
            buf = ReplayBuffer(capacity=len(batch))
            for t in batch:
                buf.push(t)
            return self._trainer.update_from_buffer(buf, batch_size=len(batch))
        return {"loss": 0.0, "n": 0.0}

    def save(self, path: str) -> None:
        self._actor.save(path)

    def load(self, path: str) -> None:
        self._actor.load(path)
