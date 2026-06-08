"""FB-CAN-035 execution feedback memory."""

from __future__ import annotations

from datetime import UTC, datetime

from app.contracts.execution_guidance import ExecutionFeedback
from data_plane.memory.execution_feedback_memory import (
    decision_quality_penalty_from_bucket,
    execution_feedback_snapshot_from_memory,
    merge_memory_into_feature_row,
    update_execution_feedback_memory,
)


def test_memory_ema_and_penalty_increases_with_bad_fills():
    st: dict[str, dict[str, float]] = {}
    ts = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    for _ in range(5):
        update_execution_feedback_memory(
            "BTC-USD",
            ExecutionFeedback(
                fill_ratio=0.7,
                realized_slippage_bps=90.0,
                fill_latency_ms=200.0,
                venue_quality_score=0.4,
            ),
            state=st,
        )
    b = st["BTC-USD"]
    assert b["slip_ema_bps"] > 0
    p = decision_quality_penalty_from_bucket(b)
    assert p > 0.1
    snap = execution_feedback_snapshot_from_memory(
        symbol="BTC-USD",
        mid_price=50_000.0,
        data_timestamp=ts,
        bucket=b,
    )
    assert snap.fill_ratio <= 1.0
    merged = merge_memory_into_feature_row({"close": 1.0}, b)
    assert "canonical_exec_quality_penalty" in merged


def test_deterministic_repeated_updates():
    st: dict[str, dict[str, float]] = {}
    fb = ExecutionFeedback(
        fill_ratio=1.0,
        realized_slippage_bps=10.0,
        fill_latency_ms=50.0,
        venue_quality_score=0.9,
    )
    for _ in range(3):
        update_execution_feedback_memory("ETH-USD", fb, state=st)
    b1 = dict(st["ETH-USD"])
    st2: dict[str, dict[str, float]] = {}
    for _ in range(3):
        update_execution_feedback_memory("ETH-USD", fb, state=st2)
    b2 = st2["ETH-USD"]
    assert abs(b1["slip_ema_bps"] - b2["slip_ema_bps"]) < 1e-9
