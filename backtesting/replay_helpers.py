"""Helpers for canonical replay execution models (FB-CAN-009)."""

from __future__ import annotations

from app.contracts.execution_guidance import ExecutionFeedback
from app.contracts.replay_events import ExecutionModelProfile
from data_plane.memory.execution_feedback_memory import update_execution_feedback_memory


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
