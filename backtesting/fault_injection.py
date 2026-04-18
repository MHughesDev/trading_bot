"""Fault injection profiles for replay (FB-CAN-009, APEX replay spec §8)."""

from __future__ import annotations

from copy import deepcopy
from datetime import UTC, datetime, timedelta
from typing import Any


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

    if profile.get("stale_data_seconds"):
        sec = float(profile["stale_data_seconds"])
        base = dt or datetime.now(UTC)
        if base.tzinfo is None:
            base = base.replace(tzinfo=UTC)
        dt = base - timedelta(seconds=sec)
        reasons.append("stale_data_injection")

    for key in profile.get("drop_feature_keys", []) or []:
        row.pop(str(key), None)
        reasons.append(f"missing_field:{key}")

    if profile.get("confidence_corruption_scale"):
        s = float(profile["confidence_corruption_scale"])
        for k in list(row.keys()):
            if "rsi" in k.lower() or "conf" in k.lower():
                row[k] = float(row[k]) * s
        reasons.append("corrupted_confidence_injection")

    return row, sp, dt, reasons
