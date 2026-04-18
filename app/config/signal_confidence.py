"""Canonical per-signal confidence / decay families (FB-CAN-032).

See APEX_Canonical_Configuration_Spec_v1_0.md §5 and §14. YAML lives under
``apex_canonical.domains.signal_confidence`` and ``apex_canonical.domains.feature_families``.
"""

from __future__ import annotations

import math
from typing import Any

from pydantic import BaseModel, Field, model_validator

# Spec §5.2 + §14 — minimum independent families (options_context covers “options” slot).
REQUIRED_SIGNAL_FAMILIES: tuple[str, ...] = (
    "market_microstructure",
    "funding",
    "open_interest",
    "basis",
    "cross_exchange_divergence",
    "liquidation_structure",
    "options_context",
    "stablecoin_flow_proxy",
    "execution_feedback",
    "novelty",
    "heat_components",
)


class SignalFamilyParams(BaseModel):
    """Per-family decay / confidence parameters (spec §5.1)."""

    model_config = {"extra": "ignore"}

    enabled: bool = True
    base_confidence_floor: float = Field(0.0, ge=0.0, le=1.0)
    base_confidence_cap: float = Field(1.0, ge=0.0, le=1.0)
    freshness_floor: float = Field(0.0, ge=0.0, le=1.0)
    freshness_cap: float = Field(1.0, ge=0.0, le=1.0)
    decay_lambda: float = Field(0.0, ge=0.0)
    latency_penalty_weight: float = Field(0.0, ge=0.0)
    reliability_penalty_weight: float = Field(0.0, ge=0.0)

    @model_validator(mode="after")
    def caps_ge_floors(self) -> SignalFamilyParams:
        if self.base_confidence_cap < self.base_confidence_floor:
            raise ValueError("base_confidence_cap must be >= base_confidence_floor")
        if self.freshness_cap < self.freshness_floor:
            raise ValueError("freshness_cap must be >= freshness_floor")
        return self


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def validate_signal_confidence_domain(dom: dict[str, Any] | None) -> list[str]:
    """Return list of validation errors (empty if OK)."""
    errs: list[str] = []
    if not dom:
        errs.append("signal_confidence domain missing or empty")
        return errs
    for name in REQUIRED_SIGNAL_FAMILIES:
        if name not in dom:
            errs.append(f"signal_confidence missing family {name!r}")
            continue
        raw = dom[name]
        if not isinstance(raw, dict):
            errs.append(f"signal_confidence.{name} must be a mapping")
            continue
        try:
            SignalFamilyParams.model_validate(raw)
        except Exception as exc:
            errs.append(f"signal_confidence.{name}: {exc}")
    return errs


def _spread_latency_proxy(row: dict[str, float]) -> float:
    sp = row.get("spread_bps")
    if sp is None:
        sp = row.get("micro_spread_bps")
    if sp is None:
        return 0.0
    try:
        return _clip01(float(sp) / 120.0)
    except (TypeError, ValueError):
        return 0.0


def apply_signal_family_confidence(
    row: dict[str, float],
    *,
    signal_confidence: dict[str, Any],
    feature_families: dict[str, Any],
) -> dict[str, float]:
    """
    Merge per-family confidence scalars into ``row`` (copy-first: mutates the passed dict).

    Uses ``feature_freshness`` / ``feature_reliability`` when present; otherwise defaults.
    Disabled families get ``signal_confidence_<family>`` = 0.0 for replay visibility.
    """
    out = row
    ff = feature_families or {}
    sc = signal_confidence or {}

    fresh_in = _clip01(float(out.get("feature_freshness", 0.92)))
    rel_in = _clip01(float(out.get("feature_reliability", 0.88)))

    for name in REQUIRED_SIGNAL_FAMILIES:
        key = f"signal_confidence_{name}"
        fam_gate = ff.get(name)
        if isinstance(fam_gate, dict) and fam_gate.get("enabled") is False:
            out[key] = 0.0
            continue

        raw = sc.get(name, {})
        if not isinstance(raw, dict):
            out[key] = 0.0
            continue
        try:
            p = SignalFamilyParams.model_validate(raw)
        except Exception:
            out[key] = 0.0
            continue
        if not p.enabled:
            out[key] = 0.0
            continue

        eff_fresh = _clip01(min(max(fresh_in, p.freshness_floor), p.freshness_cap))
        lat = _spread_latency_proxy(out)
        # Staleness → decay toward floor (deterministic, replay-friendly)
        staleness = 1.0 - eff_fresh
        decay_factor = math.exp(-p.decay_lambda * staleness) if p.decay_lambda > 0 else 1.0
        rel_term = rel_in
        core = eff_fresh * rel_term * decay_factor
        raw_score = p.base_confidence_floor + (p.base_confidence_cap - p.base_confidence_floor) * core
        raw_score -= p.latency_penalty_weight * lat
        raw_score -= p.reliability_penalty_weight * (1.0 - rel_term)
        out[key] = _clip01(raw_score)

    # Blend aggregate from per-family scores (deterministic; complements FB-CAN-016 heuristic when present)
    fam_vals = [float(out[f"signal_confidence_{n}"]) for n in REQUIRED_SIGNAL_FAMILIES if f"signal_confidence_{n}" in out]
    if fam_vals:
        agg = sum(fam_vals) / len(fam_vals)
        prev = out.get("signal_confidence_aggregate")
        if prev is not None:
            try:
                agg = 0.5 * float(prev) + 0.5 * agg
            except (TypeError, ValueError):
                pass
        out["signal_confidence_aggregate"] = _clip01(agg)

    return out


__all__ = [
    "REQUIRED_SIGNAL_FAMILIES",
    "SignalFamilyParams",
    "apply_signal_family_confidence",
    "validate_signal_confidence_domain",
]
