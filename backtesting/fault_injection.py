"""Fault injection profiles for replay (FB-CAN-009, FB-CAN-037; APEX replay spec §8)."""

from __future__ import annotations

from copy import deepcopy
from datetime import UTC, datetime, timedelta
from typing import Any


def _clip(x: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, float(x)))


def apply_fault_injection(
    *,
    feature_row: dict[str, float],
    spread_bps: float,
    data_timestamp: datetime | None,
    profile: dict[str, Any],
) -> tuple[dict[str, float], float, datetime | None, list[str]]:
    """Return (features, spread_bps, data_timestamp, fault_reason_codes)."""
    reasons: list[str] = []
    row = deepcopy(feature_row)
    sp = float(spread_bps)
    dt = data_timestamp

    if profile.get("spread_widen_mult"):
        m = float(profile["spread_widen_mult"])
        sp *= m
        reasons.append("spread_widening_injection")

    delay_sec = profile.get("delayed_data_seconds")
    if delay_sec:
        sec = float(delay_sec)
        base = dt or datetime.now(UTC)
        if base.tzinfo is None:
            base = base.replace(tzinfo=UTC)
        dt = base + timedelta(seconds=sec)
        reasons.append("delayed_data_injection")

    stale_sec = float(profile.get("stale_data_seconds", 0) or 0)
    latency_sec = float(profile.get("latency_delay_seconds", 0) or 0)
    back_total = stale_sec + latency_sec
    if back_total > 0:
        base = dt or datetime.now(UTC)
        if base.tzinfo is None:
            base = base.replace(tzinfo=UTC)
        dt = base - timedelta(seconds=back_total)
        if stale_sec > 0:
            reasons.append("stale_data_injection")
        if latency_sec > 0:
            reasons.append("latency_stress_injection")

    for key in profile.get("drop_feature_keys", []) or []:
        row.pop(str(key), None)
        reasons.append(f"missing_field:{key}")

    if profile.get("confidence_corruption_scale"):
        s = float(profile["confidence_corruption_scale"])
        for k in list(row.keys()):
            if "rsi" in k.lower() or "conf" in k.lower():
                row[k] = float(row[k]) * s
        reasons.append("corrupted_confidence_injection")

    vs = profile.get("venue_degradation_scale")
    if vs is not None:
        v = _clip(float(vs), 0.0, 1.0)
        prev = float(row.get("canonical_venue_degradation_ema", 0.0))
        row["canonical_venue_degradation_ema"] = max(prev, v)
        reasons.append("venue_degradation_injection")

    sfs = profile.get("structural_freshness_scale")
    if sfs is not None:
        m = _clip(float(sfs), 0.0, 1.0)
        for k in ("structural_freshness", "structural_reliability"):
            if k in row:
                row[k] = float(row[k]) * m
        reasons.append("structural_feed_stress_injection")

    srs = profile.get("structural_reliability_scale")
    if srs is not None:
        m = _clip(float(srs), 0.0, 1.0)
        if "structural_reliability" in row:
            row["structural_reliability"] = float(row["structural_reliability"]) * m
            reasons.append("structural_reliability_stress_injection")

    bias = profile.get("book_imbalance_bias")
    if bias is not None:
        b = _clip(float(bias), -1.0, 1.0)
        row["rsi_14"] = 50.0 + 50.0 * b
        reasons.append("book_imbalance_injection")

    slip_add = profile.get("execution_slippage_bps_add")
    if slip_add is not None:
        cur = float(row.get("canonical_exec_slippage_bps", 0.0))
        row["canonical_exec_slippage_bps"] = cur + float(slip_add)
        reasons.append("execution_slippage_stress_injection")

    fr_mult = profile.get("execution_fill_ratio_mult")
    if fr_mult is not None:
        cur = float(row.get("canonical_exec_fill_ratio", 1.0))
        row["canonical_exec_fill_ratio"] = _clip(cur * float(fr_mult), 0.0, 1.0)
        reasons.append("execution_fill_stress_injection")

    lds = profile.get("liquidity_depth_scale")
    if lds is not None:
        m = _clip(float(lds), 0.0, 1.0)
        if "volume" in row:
            row["volume"] = float(row["volume"]) * m
        row["canonical_liquidity_depth_scale"] = m
        reasons.append("liquidity_depth_collapse_injection")

    vbs = profile.get("volume_burst_scale")
    if vbs is not None:
        m = _clip(float(vbs), 0.0, 1.0)
        if "volume" in row:
            row["volume"] = float(row["volume"]) * m
        reasons.append("volume_burst_stress_injection")

    if profile.get("spread_widening_injection") and not profile.get("spread_widen_mult"):
        sp *= 2.0
        reasons.append("spread_widening_injection")

    return row, sp, dt, reasons
