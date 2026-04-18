"""Carry sleeve parameters from canonical `domains.carry` (FB-CAN-018)."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class CarrySleeveConfig:
    """§14 Carry sleeve — safe defaults; override via YAML ``apex_canonical.domains.carry``."""

    carry_enabled: bool = False
    carry_activation_requires_directional_neutrality: bool = True
    carry_max_exposure_usd: float = 5_000.0
    carry_funding_threshold: float = 0.35
    carry_independent_risk_multiplier: float = 0.35
    carry_attribution_isolation_required: bool = True
    carry_low_directional_trigger_confidence: float = 0.15

    @classmethod
    def from_canonical_domains(cls, carry: dict[str, Any] | None) -> CarrySleeveConfig:
        if not carry:
            return cls()
        def _f(key: str, default: float) -> float:
            v = carry.get(key)
            if v is None:
                return default
            return float(v)

        max_exp = carry.get("carry_max_exposure_usd")
        if max_exp is None:
            max_exp = carry.get("carry_max_exposure")
        max_exp_f = float(max_exp) if max_exp is not None else cls.carry_max_exposure_usd

        return cls(
            carry_enabled=bool(carry.get("carry_enabled", False)),
            carry_activation_requires_directional_neutrality=bool(
                carry.get("carry_activation_requires_directional_neutrality", True)
            ),
            carry_max_exposure_usd=max_exp_f,
            carry_funding_threshold=_f("carry_funding_threshold", cls.carry_funding_threshold),
            carry_independent_risk_multiplier=_f(
                "carry_independent_risk_multiplier", cls.carry_independent_risk_multiplier
            ),
            carry_attribution_isolation_required=bool(
                carry.get("carry_attribution_isolation_required", True)
            ),
            carry_low_directional_trigger_confidence=_f(
                "carry_low_directional_trigger_confidence",
                cls.carry_low_directional_trigger_confidence,
            ),
        )
