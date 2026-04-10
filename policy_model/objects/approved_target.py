"""ApprovedTarget — human policy spec §8.8."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass
class ApprovedTarget:
    approved: bool
    approved_target_fraction: float
    rejection_reasons: list[str]
    clamp_reasons: list[str]
    risk_diagnostics: dict[str, Any]
