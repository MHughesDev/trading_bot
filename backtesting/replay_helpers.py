"""Helpers for canonical replay execution models (FB-CAN-009)."""

from __future__ import annotations

from typing import Any

from app.contracts.execution_guidance import ExecutionFeedback
from app.contracts.replay_events import ExecutionModelProfile
from data_plane.memory.execution_feedback_memory import update_execution_feedback_memory


def patch_execution_feedback_event_with_partial_fill(
    events: list[dict[str, Any]],
    *,
    partial_fill_reconciliation: dict[str, Any],
) -> None:
    """Attach FB-CAN-048 reconciliation to the last execution_feedback_event in the list."""
    for ev in reversed(events):
        if ev.get("event_family") == "execution_feedback_event":
            pl = dict(ev.get("payload") or {})
            pl["partial_fill_reconciliation"] = partial_fill_reconciliation
            ev["payload"] = pl
            return


def scaled_order_quantity_for_fill_ratio(qty: Any, fill_ratio: float) -> Any:
    """Apply simulated fill ratio to intended order quantity (Decimal in, Decimal out)."""
    from decimal import Decimal

    q = qty if isinstance(qty, Decimal) else Decimal(str(qty))
    fr = max(0.0, min(1.0, float(fill_ratio)))
    return q * Decimal(str(fr))


def execution_profile_slippage_multiplier(profile: ExecutionModelProfile | str) -> float:
    p = str(profile)
    if p == "optimistic":
        return 0.85
    if p == "stress":
        return 1.35
    if p == "cascade_stress":
        return 1.6
    return 1.0


def execution_profile_fill_ratio(profile: ExecutionModelProfile | str) -> float:
    p = str(profile)
    if p == "optimistic":
        return 1.0
    if p == "stress":
        return 0.92
    if p == "cascade_stress":
        return 0.75
    return 1.0


def remaining_edge_and_exec_confidence_for_partial_fill(risk_out: Any) -> tuple[float, float]:
    """FB-CAN-048: remaining edge + exec confidence for reconcile_partial_fill_record."""
    rem = 0.01
    ec = float(getattr(risk_out, "risk_execution_confidence", None) or 0.72)
    dr = getattr(risk_out, "last_decision_record", None)
    if isinstance(dr, dict):
        diag = dr.get("diagnostics") or {}
        eg = diag.get("execution_guidance_preview")
        if isinstance(eg, dict):
            if eg.get("remaining_edge") is not None:
                rem = float(eg["remaining_edge"])
            if eg.get("execution_confidence") is not None:
                ec = float(eg["execution_confidence"])
    return rem, ec


def execution_feedback_from_simulated_fill(
    *,
    symbol: str,
    mid_price: float,
    fill_price: float,
    fill_ratio: float,
    latency_ms: float,
    exec_state: dict[str, dict[str, float]],
) -> None:
    """Update per-symbol execution memory from deterministic replay fill (FB-CAN-035)."""
    mid = max(float(mid_price), 1e-12)
    px = float(fill_price)
    slip_bps = abs(px - mid) / mid * 10_000.0
    vq = max(0.05, min(1.0, 1.0 - min(1.0, slip_bps / 200.0)))
    fb = ExecutionFeedback(
        fill_ratio=float(fill_ratio),
        realized_slippage_bps=float(slip_bps),
        fill_latency_ms=float(latency_ms),
        venue_quality_score=float(vq),
        partial_fill_flag=float(fill_ratio) < 0.999,
        adapter="replay_sim",
    )
    update_execution_feedback_memory(symbol, fb, state=exec_state)
