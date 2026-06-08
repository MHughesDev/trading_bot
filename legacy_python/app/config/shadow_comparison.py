"""Canonical shadow comparison policy (FB-CAN-038).

Validated from ``apex_canonical.domains.shadow_comparison`` in default.yaml.
"""

from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field


class ShadowComparisonThresholds(BaseModel):
    """Max allowed divergence *rates* (0–1) for promotion-style checks."""

    model_config = ConfigDict(extra="ignore")

    trigger_divergence_max: float = Field(0.05, ge=0.0, le=1.0)
    candidate_divergence_max: float = Field(0.08, ge=0.0, le=1.0)
    auction_divergence_max: float = Field(0.08, ge=0.0, le=1.0)
    suppression_divergence_max: float = Field(0.05, ge=0.0, le=1.0)
    trade_intent_divergence_max: float = Field(0.06, ge=0.0, le=1.0)


class ShadowProbationConfig(BaseModel):
    """Minimum replay depth before a shadow comparison counts as 'probation complete'."""

    model_config = ConfigDict(extra="ignore")

    min_bars: int = Field(200, ge=1, le=1_000_000)


class ShadowRollbackCriteria(BaseModel):
    """Severe breach multipliers vs thresholds (structured release evidence)."""

    model_config = ConfigDict(extra="ignore")

    severe_rate_multiplier: float = Field(1.5, ge=1.0, le=10.0)


class ShadowComparisonPolicy(BaseModel):
    """Full policy bag stored under ``domains.shadow_comparison``."""

    model_config = ConfigDict(extra="ignore")

    enabled: bool = True
    thresholds: ShadowComparisonThresholds = Field(default_factory=ShadowComparisonThresholds)
    probation: ShadowProbationConfig = Field(default_factory=ShadowProbationConfig)
    rollback: ShadowRollbackCriteria = Field(default_factory=ShadowRollbackCriteria)


def validate_shadow_comparison_domain(raw: dict[str, object] | None) -> list[str]:
    """Return validation error strings for CI gates."""
    if raw is None:
        return ["shadow_comparison domain missing"]
    if not isinstance(raw, dict):
        return ["shadow_comparison must be a mapping"]
    try:
        ShadowComparisonPolicy.model_validate(raw)
    except Exception as e:
        return [f"shadow_comparison: {e}"]
    return []


def shadow_policy_from_settings(settings: object) -> ShadowComparisonPolicy:
    """Read policy from resolved canonical bundle, or defaults."""
    try:
        dom = getattr(settings, "canonical", None)
        if dom is None:
            return ShadowComparisonPolicy()
        bag = getattr(dom, "domains", None)
        if bag is None:
            return ShadowComparisonPolicy()
        raw = getattr(bag, "shadow_comparison", None)
        if isinstance(raw, dict) and raw:
            return ShadowComparisonPolicy.model_validate(raw)
    except Exception:
        pass
    return ShadowComparisonPolicy()
