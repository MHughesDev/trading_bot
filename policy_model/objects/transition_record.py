"""TransitionRecord — human policy spec §8.10."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from policy_model.objects.policy_action import PolicyAction
from policy_model.objects.policy_observation import PolicyObservation


@dataclass
class TransitionRecord:
    observation_t: PolicyObservation
    action_t: PolicyAction
    reward_t: float
    observation_t1: PolicyObservation
    done: bool
    info: dict[str, Any]
