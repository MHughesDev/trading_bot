"""Advantage-weighted actor-critic training for the NumPy policy (FB-PL-PG2).

A real (torch-free) offline update: the trainable :class:`ValueCritic` provides a TD(0)
baseline, and the actor is regressed toward the *taken* action weighted by the exponential
advantage (advantage-weighted regression / MARWIL-style). This replaces the prior pseudo-loss
that merely shrank actions. A full PyTorch PPO/SAC variant can live behind the torch extra.
"""

from __future__ import annotations

from pathlib import Path

import numpy as np

from legacy.decision_pipeline.policy_model.policy.critic import ValueCritic
from legacy.decision_pipeline.policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from training_pipeline.policy_training.buffer import ReplayBuffer


class ActorCriticTrainer:
    def __init__(
        self,
        policy: MultiBranchMLPPolicy | None = None,
        *,
        lr: float = 0.01,
        critic: ValueCritic | None = None,
        critic_lr: float = 0.05,
        gamma: float = 0.99,
        beta: float = 1.0,
    ) -> None:
        self.policy = policy or MultiBranchMLPPolicy(seed=0)
        self.critic = critic or ValueCritic(seed=0)
        self._lr = lr
        self._critic_lr = critic_lr
        self._gamma = gamma
        self._beta = beta

    def update_from_buffer(self, buffer: ReplayBuffer, batch_size: int = 32) -> dict[str, float]:
        batch = buffer.sample_batch(batch_size)
        if not batch:
            return {"loss": 0.0, "actor_loss": 0.0, "critic_loss": 0.0, "n": 0.0}
        n = len(batch)

        # Critic TD(0) targets + advantages; the critic is updated toward each target.
        advantages: list[float] = []
        critic_sq = 0.0
        for t in batch:
            v = self.critic.forward(t.obs)
            v_next = 0.0 if t.done else self.critic.forward(t.next_obs)
            target = float(t.reward) + self._gamma * v_next
            advantages.append(target - v)
            critic_sq += self.critic.update(t.obs, target, lr=self._critic_lr)

        adv = np.asarray(advantages, dtype=np.float64)
        weights = np.exp(np.clip(self._beta * adv, -10.0, 10.0))
        weights = weights / (float(weights.mean()) + 1e-8)  # normalized AWR weights

        # Advantage-weighted regression of the actor toward the taken actions.
        grads = {
            "w_f": np.zeros_like(self.policy._w_f),
            "w_p": np.zeros_like(self.policy._w_p),
            "w_e": np.zeros_like(self.policy._w_e),
            "w_r": np.zeros_like(self.policy._w_r),
            "w_out": np.zeros_like(self.policy._w_out),
        }
        actor_sq = 0.0
        for t, wi in zip(batch, weights, strict=True):
            a, cache = self.policy.forward_with_cache(t.obs)
            a_taken = float(t.action.target_exposure)
            actor_sq += (a - a_taken) ** 2
            d_loss_d_a = 2.0 * float(wi) * (a - a_taken) / n
            g = self.policy.backward(cache, d_loss_d_a)
            for k in grads:
                grads[k] += g[k]
        self.policy.apply_grads(grads, self._lr)

        return {
            "loss": actor_sq / n,
            "actor_loss": actor_sq / n,
            "critic_loss": critic_sq / n,
            "n": float(n),
        }

    def save(self, path: str | Path) -> None:
        p = Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(
            p,
            w_f=self.policy._w_f,
            w_p=self.policy._w_p,
            w_e=self.policy._w_e,
            w_r=self.policy._w_r,
            w_out=self.policy._w_out,
            critic_w=self.critic._w,
        )

    def load(self, path: str | Path) -> None:
        p = Path(path)
        with np.load(p, allow_pickle=False) as z:
            if "w_f" in z:
                self.policy._w_f = np.asarray(z["w_f"])
                self.policy._w_p = np.asarray(z["w_p"])
                self.policy._w_e = np.asarray(z["w_e"])
                self.policy._w_r = np.asarray(z["w_r"])
                self.policy._w_out = np.asarray(z["w_out"])
            if "critic_w" in z:
                self.critic._w = np.asarray(z["critic_w"])


def walk_forward_episode_slices(total_steps: int, *, window: int, step: int) -> list[tuple[int, int]]:
    """Indices [start, end) for walk-forward RL training."""
    out: list[tuple[int, int]] = []
    start = 0
    while start + window <= total_steps:
        out.append((start, start + window))
        start += step
    return out
