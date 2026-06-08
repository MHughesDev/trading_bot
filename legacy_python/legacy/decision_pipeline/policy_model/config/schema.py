"""Policy configuration schema placeholder — human policy spec §6."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class PolicyConfig:
    """Minimal first-version defaults (spec §33)."""

    continuous_target_exposure: bool = True
    deterministic_risk_gate: bool = True


__all__ = ["PolicyConfig"]
