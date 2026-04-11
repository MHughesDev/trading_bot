"""Actor-critic training stub with walk-forward schedule hook (FB-PL-PG2)."""

from __future__ import annotations

from pathlib import Path

import numpy as np

from policy_model.policy.mlp_actor import MultiBranchMLPPolicy
from policy_model.training.buffer import ReplayBuffer


class ActorCriticTrainer:
    """
    Minimal on-policy style update using buffer samples (NumPy policy).

    Full PyTorch actor-critic belongs behind optional torch extra.
    """

    def __init__(self, policy: MultiBranchMLPPolicy | None = None, *, lr: float = 0.01) -> None:
        self.policy = policy or MultiBranchMLPPolicy(seed=0)
        self._lr = lr

    def update_from_buffer(self, buffer: ReplayBuffer, batch_size: int = 32) -> dict[str, float]:
        batch = buffer.sample_batch(batch_size)
        if not batch:
            return {"loss": 0.0, "n": 0.0}
        # Pseudo-loss: encourage small actions (stability placeholder)
        losses = []
        for t in batch:
            a = self.policy.select_action(t.obs, deterministic=True).target_exposure
            losses.append(float(a * a))
        loss = float(np.mean(losses)) * self._lr
        return {"loss": loss, "n": float(len(batch))}

    def save(self, path: str | Path) -> None:
        p = Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        np.savez_compressed(p, seed=0)

    def load(self, path: str | Path) -> None:
        _ = Path(path)


def walk_forward_episode_slices(total_steps: int, *, window: int, step: int) -> list[tuple[int, int]]:
    """Indices [start, end) for walk-forward RL training."""
    out: list[tuple[int, int]] = []
    start = 0
    while start + window <= total_steps:
        out.append((start, start + window))
        start += step
    return out
