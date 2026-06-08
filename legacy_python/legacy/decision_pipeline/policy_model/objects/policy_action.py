"""PolicyAction — human policy spec §8.6."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass
class PolicyAction:
    target_exposure: float
    action_diagnostics: dict[str, Any] = field(default_factory=dict)
