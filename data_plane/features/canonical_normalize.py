"""Canonical feature naming, freshness/reliability, and completeness (FB-CAN-016).

Maps pipeline outputs to APEX-normalized field names and attaches
``feature_freshness``, ``feature_reliability``, ``signal_confidence_aggregate``,
and ``canonical_snapshot_complete`` per
``APEX_Decision_Service_Feature_Schema_and_Data_Contracts_v1_0.md`` §4.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

# Bump when normalization rules change (stored on feature rows).
CANONICAL_NORMALIZATION_VERSION = 1

# Primary return keys the decision path prefers (alias from ``ret_*``).
_RETURN_ALIASES: tuple[tuple[str, str], ...] = (
    ("ret_1", "return_1"),
    ("ret_3", "return_3"),
    ("ret_5", "return_5"),
    ("ret_15", "return_15"),
)


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def completeness_score(row: dict[str, float]) -> float:
    """1.0 when core families present; degrades smoothly when not."""
    has_price = "close" in row and float(row.get("close", 0.0)) > 0
    # ret_1 or return_1
    has_ret = any(k in row for k in ("ret_1", "return_1"))
    has_vol = "volume" in row
    has_rsi = "rsi_14" in row
    has_atr = "atr_14" in row
    parts = [has_price, has_ret, has_vol, has_rsi, has_atr]
    return sum(1.0 for p in parts if p) / max(len(parts), 1)


def feature_freshness_from_age(bar_age_seconds: float | None, *, stale_seconds: float = 120.0) -> float:
    """§4.4 — 1.0 fresh, 0.0 at/above stale horizon."""
    if bar_age_seconds is None or bar_age_seconds < 0:
        return 1.0
    return _clip01(1.0 - float(bar_age_seconds) / max(stale_seconds, 1e-6))


def feature_reliability_heuristic(row: dict[str, float]) -> float:
    """§4.3 — higher when volume and ATR indicate usable signal density."""
    close = max(float(row.get("close", 0.0) or 0.0), 1e-12)
    vol = max(float(row.get("volume", 0.0) or 0.0), 0.0)
    atr = max(float(row.get("atr_14", 0.0) or 0.0), 0.0)
    vol_rel = _clip01(vol / (close * 1e-5 + 1e3))
    atr_rel = _clip01(atr / close * 80.0)
    return _clip01(0.45 + 0.35 * vol_rel + 0.2 * atr_rel)


def normalize_feature_row(
    row: dict[str, float],
    *,
    bar_age_seconds: float | None = None,
    stale_data_seconds: float = 120.0,
) -> dict[str, float]:
    """
    Return a **copy** with canonical aliases and freshness/reliability fields.

    - ``ret_*`` → ``return_*`` when the latter is missing.
    - ``micro_spread_bps`` copied to ``spread_bps_feature`` if missing (ingest overlay).
    """
    out = dict(row)

    for src, dst in _RETURN_ALIASES:
        if dst not in out and src in out:
            out[dst] = float(out[src])

    if "spread_bps_feature" not in out and "micro_spread_bps" in out:
        out["spread_bps_feature"] = float(out["micro_spread_bps"])

    fresh = feature_freshness_from_age(bar_age_seconds, stale_seconds=stale_data_seconds)
    rel = feature_reliability_heuristic(out)
    complete = completeness_score(out)
    conf = _clip01(0.5 * fresh + 0.35 * rel + 0.15 * complete)

    out["feature_freshness"] = fresh
    out["feature_reliability"] = rel
    out["canonical_snapshot_complete"] = complete
    out["signal_confidence_aggregate"] = conf
    out["canonical_normalization_version"] = float(CANONICAL_NORMALIZATION_VERSION)
    # Structural bundle proxies (until perp/OI ingest is wired)
    out["structural_freshness"] = _clip01(0.75 * fresh + 0.25 * rel)
    out["structural_reliability"] = _clip01(0.65 * rel + 0.35 * complete)
    return out


def validate_normalized_row(row: dict[str, float]) -> tuple[bool, list[str]]:
    """
    Return (ok, reasons). If ``signal_confidence_aggregate`` or completeness is too low,
    callers should apply degradation (spec: reject incomplete snapshots per policy).
    """
    reasons: list[str] = []
    sc = float(row.get("signal_confidence_aggregate", 1.0))
    cs = float(row.get("canonical_snapshot_complete", 1.0))
    if sc < 0.25:
        reasons.append("low_signal_confidence_aggregate")
    if cs < 0.4:
        reasons.append("incomplete_canonical_snapshot")
    return (len(reasons) == 0, reasons)


def normalization_diagnostics(row: dict[str, float]) -> dict[str, Any]:
    ok, reasons = validate_normalized_row(row)
    return {
        "ok": ok,
        "reasons": reasons,
        "feature_freshness": row.get("feature_freshness"),
        "feature_reliability": row.get("feature_reliability"),
        "canonical_snapshot_complete": row.get("canonical_snapshot_complete"),
        "signal_confidence_aggregate": row.get("signal_confidence_aggregate"),
    }


def bar_age_seconds_from_timestamp(
    ts: datetime | Any,
    *,
    now: datetime | None = None,
) -> float | None:
    """Age in seconds for freshness; None if ts invalid."""
    if ts is None:
        return None
    if not isinstance(ts, datetime):
        return None
    t = ts if ts.tzinfo else ts.replace(tzinfo=UTC)
    n = now or datetime.now(UTC)
    if n.tzinfo is None:
        n = n.replace(tzinfo=UTC)
    return max(0.0, (n - t).total_seconds())
