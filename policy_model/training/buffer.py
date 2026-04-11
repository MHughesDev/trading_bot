"""Simple replay buffer for RL transitions (FB-PL-PG2 / FB-PL-CORE deliverable)."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass
from typing import Any

from policy_model.objects import PolicyAction, PolicyObservation


@dataclass
class Transition:
    obs: PolicyObservation
    action: PolicyAction
    reward: float
    next_obs: PolicyObservation
    done: bool
    info: dict[str, Any]


class ReplayBuffer:
    def __init__(self, capacity: int = 10_000) -> None:
        self._buf: deque[Transition] = deque(maxlen=capacity)

    def push(self, t: Transition) -> None:
        self._buf.append(t)

    def __len__(self) -> int:
        return len(self._buf)

    def sample_batch(self, n: int) -> list[Transition]:
        import random

        if len(self._buf) == 0:
            return []
        n = min(n, len(self._buf))
        return random.sample(list(self._buf), n)
