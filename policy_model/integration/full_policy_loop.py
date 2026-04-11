"""
End-to-end RL policy loop stub (FB-PL-CORE; full spec RL policy training: FB-PL-P0): env → buffer → trainer.

Uses `ReplayPolicyEnvironment`, `MultiBranchMLPPolicy`, `ReplayBuffer`, `ActorCriticTrainer`.
"""

from __future__ import annotations

import numpy as np

from policy_model.env.replay_env import ReplayPolicyEnvironment
from policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from policy_model.training.actor_critic import ActorCriticTrainer
from policy_model.training.buffer import ReplayBuffer, Transition


def run_stub_episode(closes: np.ndarray, *, max_steps: int = 5) -> dict[str, float]:
    env = ReplayPolicyEnvironment(closes)
    pol = MultiBranchMLPPolicy(seed=1)
    buf = ReplayBuffer(capacity=1000)
    trainer = ActorCriticTrainer(policy=pol)
    obs = env.reset()
    total_r = 0.0
    for _ in range(min(max_steps, len(closes) - 1)):
        a = pol.select_action(obs, deterministic=True)
        next_obs, r, done, _info = env.step(a)
        buf.push(Transition(obs, a, r, next_obs, done, {}))
        total_r += r
        obs = next_obs
        if done:
            break
    metrics = trainer.update_from_buffer(buf, batch_size=min(8, len(buf)))
    return {"return": total_r, **metrics}
