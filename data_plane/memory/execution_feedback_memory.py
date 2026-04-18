"""Slow-moving execution feedback memory (FB-CAN-035).

EMAs over realized slippage, fill ratio, latency, and venue degradation; surfaces
:class:`ExecutionFeedbackSnapshot` for the decision boundary and deterministic replay.
"""

from __future__ import annotations

import uuid
from datetime import UTC, datetime
from typing import Any

from app.contracts.decision_snapshots import ExecutionFeedbackSnapshot, OrderStyleUsed
from app.contracts.execution_guidance import ExecutionFeedback
from execution.execution_logic import apply_execution_feedback

# Slow-moving updates (same order of magnitude as a few dozen bars)
_MEMORY_ALPHA = 0.08


def _clip01(x: float) -> float:
    return max(0.0, min(1.0, float(x)))


def update_execution_feedback_memory(
    symbol: str,
    feedback: ExecutionFeedback,
    *,
    state: dict[str, dict[str, float]],
) -> dict[str, float]:
    """Merge post-trade feedback into per-symbol memory (deterministic EMAs)."""
    prev = dict(state.get(symbol, {}))
    base = apply_execution_feedback(symbol, feedback, state=state)
    out: dict[str, float] = {**prev, **{k: float(v) for k, v in base.items()}}

    slip_obs = abs(float(feedback.realized_slippage_bps))
    slip_ema = _MEMORY_ALPHA * slip_obs + (1.0 - _MEMORY_ALPHA) * float(
        prev.get("slip_ema_bps", slip_obs)
    )
    fill_obs = float(feedback.fill_ratio)
    fill_ema = _MEMORY_ALPHA * fill_obs + (1.0 - _MEMORY_ALPHA) * float(
        prev.get("fill_ratio_ema", fill_obs)
    )
    lat_obs = (
        float(feedback.fill_latency_ms)
        if feedback.fill_latency_ms is not None
        else float(prev.get("latency_ms_ema", 45.0))
    )
    lat_ema = _MEMORY_ALPHA * lat_obs + (1.0 - _MEMORY_ALPHA) * float(
        prev.get("latency_ms_ema", lat_obs)
    )
    vdeg_obs = 1.0 - _clip01(float(feedback.venue_quality_score))
    vdeg_ema = _MEMORY_ALPHA * vdeg_obs + (1.0 - _MEMORY_ALPHA) * float(
        prev.get("venue_degradation_ema", vdeg_obs)
    )
    out["slip_ema_bps"] = slip_ema
    out["fill_ratio_ema"] = fill_ema
    out["latency_ms_ema"] = lat_ema
    out["venue_degradation_ema"] = vdeg_ema
    out["memory_tick_count"] = float(prev.get("memory_tick_count", 0.0)) + 1.0
    state[symbol] = out
    return out


def decision_quality_penalty_from_bucket(bucket: dict[str, float]) -> float:
    """Scalar [0,1] for auction / feature injection (higher = worse past execution)."""
    slip = float(bucket.get("slip_ema_bps", 0.0))
    fill_short = max(0.0, 1.0 - float(bucket.get("fill_ratio_ema", 1.0)))
    lat = float(bucket.get("latency_ms_ema", 0.0))
    vdeg = float(bucket.get("venue_degradation_ema", 0.0))
    trust_short = max(0.0, 1.0 - float(bucket.get("execution_trust", 0.75)))
    p_slip = _clip01(slip / 120.0)
    p_lat = _clip01(lat / 800.0)
    return _clip01(0.28 * p_slip + 0.22 * fill_short + 0.18 * p_lat + 0.2 * vdeg + 0.12 * trust_short)


def execution_feedback_snapshot_from_memory(
    *,
    symbol: str,
    mid_price: float,
    data_timestamp: datetime | None,
    bucket: dict[str, float],
) -> ExecutionFeedbackSnapshot:
    """Build boundary snapshot from memory (replay/live parity)."""
    ts = data_timestamp if data_timestamp is not None else datetime.now(UTC)
    if ts.tzinfo is None:
        ts = ts.replace(tzinfo=UTC)
    mp = float(mid_price)
    slip = float(bucket.get("slip_ema_bps", 0.0))
    fill = _clip01(float(bucket.get("fill_ratio_ema", 1.0)))
    lat = max(0.0, float(bucket.get("latency_ms_ema", 0.0)))
    vq = _clip01(1.0 - float(bucket.get("venue_degradation_ema", 0.0)))
    ec = _clip01(
        0.34 * (1.0 - slip / 150.0)
        + 0.33 * fill
        + 0.33 * (1.0 - min(1.0, lat / 600.0))
    )
    return ExecutionFeedbackSnapshot(
        feedback_id=f"mem-{uuid.uuid4().hex[:12]}",
        timestamp=ts,
        instrument_id=symbol,
        related_intent_id="memory",
        expected_fill_price=mp,
        realized_fill_price=mp,
        realized_slippage_bps=slip,
        fill_ratio=fill,
        fill_latency_ms=lat,
        execution_confidence_realized=ec,
        venue_quality_score=vq,
        partial_fill_flag=fill < 0.999,
        order_style_used=OrderStyleUsed.MARKET,
    )


def merge_memory_into_feature_row(
    merged: dict[str, float],
    bucket: dict[str, float],
) -> dict[str, float]:
    """Inject canonical scalars for trigger/auction (mutates copy)."""
    out = dict(merged)
    dq = decision_quality_penalty_from_bucket(bucket)
    out["canonical_exec_quality_penalty"] = dq
    out["canonical_venue_degradation_ema"] = float(bucket.get("venue_degradation_ema", 0.0))
    out["canonical_exec_slippage_ema_bps"] = float(bucket.get("slip_ema_bps", 0.0))
    out["canonical_exec_fill_ratio_ema"] = float(bucket.get("fill_ratio_ema", 1.0))
    return out


def memory_bucket_to_diagnostic(bucket: dict[str, float]) -> dict[str, Any]:
    """JSON-safe slice for ForecastPacket diagnostics."""
    out: dict[str, Any] = {
        "decision_quality_penalty": decision_quality_penalty_from_bucket(bucket),
    }
    for k in (
        "execution_trust",
        "venue_quality",
        "slip_ema_bps",
        "fill_ratio_ema",
        "latency_ms_ema",
        "venue_degradation_ema",
        "memory_tick_count",
    ):
        if k in bucket:
            out[k] = bucket[k]
    return out


__all__ = [
    "decision_quality_penalty_from_bucket",
    "execution_feedback_snapshot_from_memory",
    "memory_bucket_to_diagnostic",
    "merge_memory_into_feature_row",
    "update_execution_feedback_memory",
]
