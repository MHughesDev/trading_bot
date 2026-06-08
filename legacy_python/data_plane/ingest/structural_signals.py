"""Structural signal ingestion helpers (FB-CAN-049).

Normalizes optional upstream fields (Kraken REST, synthetic overlays, replay fixtures)
into canonical feature-row keys aligned with
``APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md`` §6.

Raw keys supported (examples — any may be omitted):

- ``struct_funding_rate``, ``struct_funding_rate_zscore``, ``struct_funding_velocity``,
  ``struct_funding_age_seconds``
- ``struct_open_interest``, ``struct_open_interest_delta_short``, ``struct_oi_age_seconds``
- ``struct_basis_bps``, ``struct_basis_age_seconds``
- ``struct_cross_exchange_divergence``, ``struct_divergence_age_seconds``
- ``struct_liquidation_proximity_long``, ``struct_liquidation_proximity_short``,
  ``struct_liquidation_cluster_density_long``, ``struct_liquidation_cluster_density_short``,
  ``struct_liquidation_data_confidence``, ``struct_liquidation_age_seconds``

Canonical names (``funding_rate``, ``basis_bps``, …) win when both are present.
"""

from __future__ import annotations

from typing import Any

# Default staleness horizons (seconds) when age is unknown but a value exists.
_DEFAULT_STALE: dict[str, float] = {
    "funding": 3600.0,
    "open_interest": 7200.0,
    "basis": 1800.0,
    "cross_exchange_divergence": 2700.0,
    "liquidation_structure": 1200.0,
}


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def _f(x: Any) -> float | None:
    if x is None:
        return None
    try:
        return float(x)
    except (TypeError, ValueError):
        return None


def _pick(row: dict[str, float], *keys: str) -> float | None:
    for k in keys:
        if k in row:
            v = _f(row.get(k))
            if v is not None:
                return v
    return None


def _age(row: dict[str, float], key: str, *, default_if_present: float = 0.0) -> float:
    v = _f(row.get(key))
    if v is None:
        return default_if_present
    return max(0.0, float(v))


def merge_structural_signal_overlay(
    base: dict[str, float],
    overlay: dict[str, float] | None,
) -> dict[str, float]:
    """Merge overlay into base (overlay wins on key collision) then apply family normalization."""
    merged = dict(base)
    if overlay:
        merged.update(overlay)
    return apply_structural_families_from_row(merged)


def apply_structural_families_from_row(row: dict[str, float]) -> dict[str, float]:
    """
    Populate canonical structural scalars, per-family freshness, coverage, and missing flags.

    Mutates and returns the same dict for convenience (call on a copy if needed).
    """
    out = row

    # --- Funding ---
    fr = _pick(out, "funding_rate", "struct_funding_rate")
    fr_z = _pick(out, "funding_rate_zscore", "struct_funding_rate_zscore")
    fv = _pick(out, "funding_velocity", "struct_funding_velocity")
    if fr is not None:
        out["funding_rate"] = float(fr)
    if fr_z is not None:
        out["funding_rate_zscore"] = float(fr_z)
    elif fr is not None:
        # Rough z proxy when history not available (deterministic, bounded).
        out["funding_rate_zscore"] = float(max(-4.0, min(4.0, float(fr) * 400.0)))
    if fv is not None:
        out["funding_velocity"] = float(fv)

    has_funding = "funding_rate" in out
    age_f = _age(out, "struct_funding_age_seconds", default_if_present=0.0) if has_funding else None
    fresh_f = _clip01(1.0 - float(age_f or 0.0) / max(_DEFAULT_STALE["funding"], 1e-6)) if has_funding else 0.0
    out["structural_funding_freshness"] = fresh_f
    out["structural_missing_funding"] = 0.0 if has_funding else 1.0

    # --- Open interest ---
    oi = _pick(out, "open_interest", "struct_open_interest", "oi")
    oi_d = _pick(out, "open_interest_delta_short", "struct_open_interest_delta_short", "oi_delta_short")
    if oi is not None:
        out["open_interest"] = float(oi)
    if oi_d is not None:
        out["open_interest_delta_short"] = float(oi_d)

    has_oi = "open_interest" in out
    age_oi = _age(out, "struct_oi_age_seconds", default_if_present=0.0) if has_oi else None
    fresh_oi = _clip01(1.0 - float(age_oi or 0.0) / max(_DEFAULT_STALE["open_interest"], 1e-6)) if has_oi else 0.0
    out["structural_open_interest_freshness"] = fresh_oi
    out["structural_missing_open_interest"] = 0.0 if has_oi else 1.0

    # --- Basis / perp-spot ---
    bb = _pick(out, "basis_bps", "struct_basis_bps", "perp_spot_basis_bps")
    if bb is not None:
        out["basis_bps"] = float(bb)
    has_basis = "basis_bps" in out
    age_b = _age(out, "struct_basis_age_seconds", default_if_present=0.0) if has_basis else None
    fresh_b = _clip01(1.0 - float(age_b or 0.0) / max(_DEFAULT_STALE["basis"], 1e-6)) if has_basis else 0.0
    out["structural_basis_freshness"] = fresh_b
    out["structural_missing_basis"] = 0.0 if has_basis else 1.0

    # --- Cross-exchange divergence ---
    cxd = _pick(
        out,
        "cross_exchange_divergence",
        "struct_cross_exchange_divergence",
        "cex_divergence",
    )
    if cxd is not None:
        out["cross_exchange_divergence"] = float(cxd)
    has_cxd = "cross_exchange_divergence" in out
    age_x = _age(out, "struct_divergence_age_seconds", default_if_present=0.0) if has_cxd else None
    fresh_x = _clip01(1.0 - float(age_x or 0.0) / max(_DEFAULT_STALE["cross_exchange_divergence"], 1e-6))
    if has_cxd:
        out["structural_cross_exchange_freshness"] = fresh_x
    else:
        out["structural_cross_exchange_freshness"] = 0.0
    out["structural_missing_cross_exchange_divergence"] = 0.0 if has_cxd else 1.0

    # --- Liquidation structure ---
    lpl = _pick(out, "liquidation_proximity_long", "struct_liquidation_proximity_long")
    lps = _pick(out, "liquidation_proximity_short", "struct_liquidation_proximity_short")
    lcd_l = _pick(out, "liquidation_cluster_density_long", "struct_liquidation_cluster_density_long")
    lcd_s = _pick(out, "liquidation_cluster_density_short", "struct_liquidation_cluster_density_short")
    lconf = _pick(out, "liquidation_data_confidence", "struct_liquidation_data_confidence")

    has_liq = any(x is not None for x in (lpl, lps, lcd_l, lcd_s, lconf))
    if lpl is not None:
        out["liquidation_proximity_long"] = _clip01(float(lpl))
    if lps is not None:
        out["liquidation_proximity_short"] = _clip01(float(lps))
    if lcd_l is not None:
        out["liquidation_cluster_density_long"] = _clip01(float(lcd_l))
    if lcd_s is not None:
        out["liquidation_cluster_density_short"] = _clip01(float(lcd_s))
    if lconf is not None:
        out["liquidation_data_confidence"] = _clip01(float(lconf))

    age_l = _age(out, "struct_liquidation_age_seconds", default_if_present=0.0) if has_liq else None
    fresh_l = _clip01(1.0 - float(age_l or 0.0) / max(_DEFAULT_STALE["liquidation_structure"], 1e-6)) if has_liq else 0.0
    out["structural_liquidation_freshness"] = fresh_l
    out["structural_missing_liquidation_structure"] = 0.0 if has_liq else 1.0

    # Optional perp–spot score (alias of basis family when only score provided)
    psd = _pick(out, "perp_spot_divergence_score", "struct_perp_spot_divergence_score")
    if psd is not None:
        out["perp_spot_divergence_score"] = float(psd)

    # Bundle coverage / degradation hints
    miss = [
        out["structural_missing_funding"],
        out["structural_missing_open_interest"],
        out["structural_missing_basis"],
        out["structural_missing_cross_exchange_divergence"],
        out["structural_missing_liquidation_structure"],
    ]
    present = 5.0 - sum(miss)
    out["structural_family_coverage"] = _clip01(present / 5.0)
    out["structural_all_missing"] = 1.0 if present == 0.0 else 0.0

    freqs = [
        out["structural_funding_freshness"],
        out["structural_open_interest_freshness"],
        out["structural_basis_freshness"],
        out["structural_cross_exchange_freshness"],
        out["structural_liquidation_freshness"],
    ]
    out["structural_per_family_freshness_mean"] = sum(freqs) / 5.0

    return out


# Keys useful for replay / audit payloads (subset of normalized row).
STRUCTURAL_REPLAY_KEYS: tuple[str, ...] = (
    "funding_rate",
    "funding_rate_zscore",
    "funding_velocity",
    "open_interest",
    "open_interest_delta_short",
    "basis_bps",
    "cross_exchange_divergence",
    "liquidation_proximity_long",
    "liquidation_proximity_short",
    "liquidation_cluster_density_long",
    "liquidation_cluster_density_short",
    "liquidation_data_confidence",
    "perp_spot_divergence_score",
    "structural_freshness",
    "structural_reliability",
    "structural_family_coverage",
    "structural_per_family_freshness_mean",
    "structural_missing_funding",
    "structural_missing_open_interest",
    "structural_missing_basis",
    "structural_missing_cross_exchange_divergence",
    "structural_missing_liquidation_structure",
    "structural_all_missing",
)


__all__ = [
    "STRUCTURAL_REPLAY_KEYS",
    "apply_structural_families_from_row",
    "merge_structural_signal_overlay",
]
