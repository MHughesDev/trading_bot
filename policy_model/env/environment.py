"""Trading policy environment — human policy spec §29.6 `TradingPolicyEnvironment`."""

from __future__ import annotations

from typing import Any, Protocol

from policy_model.objects import PolicyAction, PolicyObservation


class TradingPolicyEnvironment(Protocol):
    def reset(self) -> PolicyObservation: ...

    def step(self, action: PolicyAction) -> tuple[PolicyObservation, float, bool, dict[str, Any]]: ...
