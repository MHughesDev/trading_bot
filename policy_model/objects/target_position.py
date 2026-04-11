"""TargetPosition — human policy spec §8.7."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class TargetPosition:
    target_fraction: float
    target_units: float | None
    target_notional: float | None
    target_leverage: float
    reason_codes: list[str]
