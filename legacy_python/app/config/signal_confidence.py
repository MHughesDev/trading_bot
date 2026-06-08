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

# Optional upstream families: availability gates + lower weight in aggregate blend (FB-CAN-050).
OPTIONAL_SIGNAL_FAMILIES: frozenset[str] = frozenset({"options_context", "stablecoin_flow_proxy"})
OPTIONAL_FAMILY_AVAILABILITY_KEY: dict[str, str] = {
    "options_context": "options_context_available",
    "stablecoin_flow_proxy": "stablecoin_flow_available",
}


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

        if name in OPTIONAL_SIGNAL_FAMILIES:
            ak = OPTIONAL_FAMILY_AVAILABILITY_KEY.get(name, "")
            if ak and float(out.get(ak, 0.0)) < 0.5:
                out[key] = _clip01(p.base_confidence_floor)
                continue

        eff_fresh = _clip01(min(max(fresh_in, p.freshness_floor), p.freshness_cap))
        if name == "options_context":
            eff_fresh = _clip01(min(eff_fresh, float(out.get("options_freshness", eff_fresh))))
        elif name == "stablecoin_flow_proxy":
            eff_fresh = _clip01(min(eff_fresh, float(out.get("stablecoin_freshness", eff_fresh))))
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

    # Blend aggregate from per-family scores (optional families down-weighted; FB-CAN-050)
    weighted: list[tuple[float, float]] = []
    for n in REQUIRED_SIGNAL_FAMILIES:
        sk = f"signal_confidence_{n}"
        if sk not in out:
            continue
        w = 0.35 if n in OPTIONAL_SIGNAL_FAMILIES else 1.0
        weighted.append((float(out[sk]), w))
    if weighted:
        num = sum(v * w for v, w in weighted)
        den = sum(w for _, w in weighted)
        agg = num / den if den > 0 else 0.0
        prev = out.get("signal_confidence_aggregate")
        if prev is not None:
            try:
                agg = 0.5 * float(prev) + 0.5 * agg
            except (TypeError, ValueError):
                pass
        out["signal_confidence_aggregate"] = _clip01(agg)

    # Explicit fallback flags when family is enabled in config but upstream has no data (FB-CAN-050)
    for name in OPTIONAL_SIGNAL_FAMILIES:
        ak = OPTIONAL_FAMILY_AVAILABILITY_KEY.get(name, "")
        fam_gate = ff.get(name)
        enabled = isinstance(fam_gate, dict) and fam_gate.get("enabled") is True
        if ak:
            miss = enabled and float(out.get(ak, 0.0)) < 0.5
            out[f"{name}_fallback_active"] = 1.0 if miss else 0.0
            if name == "stablecoin_flow_proxy":
                out["stablecoin_flow_fallback_active"] = out["stablecoin_flow_proxy_fallback_active"]

    return out


__all__ = [
    "OPTIONAL_FAMILY_AVAILABILITY_KEY",
    "OPTIONAL_SIGNAL_FAMILIES",
    "REQUIRED_SIGNAL_FAMILIES",
    "SignalFamilyParams",
    "apply_signal_family_confidence",
    "validate_signal_confidence_domain",
]
