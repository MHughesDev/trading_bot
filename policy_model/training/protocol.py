"""RL algorithm abstraction (human policy spec §19)."""

from __future__ import annotations

from typing import Any, Protocol, runtime_checkable

from policy_model.objects import PolicyAction, PolicyObservation


@runtime_checkable
class RLPolicyAlgorithm(Protocol):
    def select_action(self, obs: PolicyObservation, deterministic: bool) -> PolicyAction: ...

    def update(self, batch: Any) -> dict[str, float]: ...

    def save(self, path: str) -> None: ...

    def load(self, path: str) -> None: ...
