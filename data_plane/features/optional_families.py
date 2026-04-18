"""Optional canonical feature families: options context + stablecoin flow proxy (FB-CAN-050).

Derives availability flags, freshness, and fallback indicators for
``apex_canonical.domains.signal_confidence`` families ``options_context`` and
``stablecoin_flow_proxy``. Missing upstream data must not break decisioning; confidence
floors apply via ``apply_signal_family_confidence``.
"""

from __future__ import annotations

from typing import Any

_DEFAULT_OPTIONS_STALE = 900.0
_DEFAULT_STABLECOIN_STALE = 3600.0


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def _f(x: Any) -> float | None:
    if x is None:
        return None
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


def _age(row: dict[str, float], key: str) -> float:
    v = _f(row.get(key))
    if v is None:
        return 0.0
    return max(0.0, float(v))


def apply_options_and_stablecoin_families(row: dict[str, float]) -> dict[str, float]:
    """
    Populate options + stablecoin fields on ``row`` (mutates in place).

    - Options: ``gex_score``, ``iv_skew_score`` from canonical or ``struct_*`` aliases.
    - ``options_freshness`` / ``options_reliability`` from ages or neutral defaults when present.
    - ``options_context_available`` ∈ {0,1}; ``options_context_fallback`` = 1 when enabled upstream
      but no options fields (checked later against feature_families in confidence path).
    - Stablecoin: ``stablecoin_flow_proxy`` from canonical or ``struct_stablecoin_flow_proxy``;
      ``stablecoin_freshness``; ``stablecoin_flow_available`` ∈ {0,1}.
    """
    out = row

    gex = _f(out.get("gex_score"))
    if gex is None:
        gex = _f(out.get("struct_gex_score"))
    if gex is not None:
        out["gex_score"] = max(-1.0, min(1.0, float(gex)))

    iv = _f(out.get("iv_skew_score"))
    if iv is None:
        iv = _f(out.get("struct_iv_skew_score"))
    if iv is not None:
        out["iv_skew_score"] = max(-1.0, min(1.0, float(iv)))

    has_options = "gex_score" in out or "iv_skew_score" in out
    out["options_context_available"] = 1.0 if has_options else 0.0

    age_opt = _age(out, "struct_options_age_seconds") if has_options else None
    if has_options:
        out["options_freshness"] = _clip01(1.0 - float(age_opt or 0.0) / max(_DEFAULT_OPTIONS_STALE, 1e-6))
        rel_o = _f(out.get("options_reliability"))
        if rel_o is None:
            rel_o = _f(out.get("struct_options_reliability"))
        out["options_reliability"] = _clip01(float(rel_o)) if rel_o is not None else _clip01(0.55 + 0.45 * out["options_freshness"])
    else:
        out["options_freshness"] = 0.0
        out["options_reliability"] = 0.0

    sc = _f(out.get("stablecoin_flow_proxy"))
    if sc is None:
        sc = _f(out.get("struct_stablecoin_flow_proxy"))
    if sc is not None:
        out["stablecoin_flow_proxy"] = float(max(-1.0, min(1.0, float(sc))))

    has_sc = "stablecoin_flow_proxy" in out
    out["stablecoin_flow_available"] = 1.0 if has_sc else 0.0
    age_sc = _age(out, "struct_stablecoin_age_seconds") if has_sc else None
    if has_sc:
        out["stablecoin_freshness"] = _clip01(1.0 - float(age_sc or 0.0) / max(_DEFAULT_STABLECOIN_STALE, 1e-6))
    else:
        out["stablecoin_freshness"] = 0.0

    return out


__all__ = ["apply_options_and_stablecoin_families"]
