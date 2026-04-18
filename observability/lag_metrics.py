"""Per-domain lag histograms for canonical system health (FB-CAN-072).

``APEX_Monitoring_and_Alerting_Spec_v1_0.md`` §4.1 calls out event processing lag, decision cycle
duration, and execution latency. This module decomposes:

- **data_ingestion** — max of feed-last-message age and decision ``data_timestamp`` age (seconds),
  matching the stale-input semantics in ``risk_engine.engine`` (event lag).
- **decision** — wall-clock seconds for ``pipeline.step`` + ``risk_engine.evaluate`` only (processing
  lag inside the hot path before record emission).
- **execution_feedback** — slow-moving EMA fill latency from ``execution_feedback_memory`` (seconds).

End-to-end tick wall time remains ``tb_decision_latency_seconds`` in ``observability/metrics.py``.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any

from prometheus_client import Histogram

CANONICAL_LAG_SECONDS = Histogram(
    "tb_canonical_lag_seconds",
    "Per-domain lag in seconds (FB-CAN-072): data_ingestion=max(feed,data_ts age); "
    "decision=pipeline+risk wall time; execution_feedback=memory latency EMA",
    ["symbol", "domain"],
    buckets=(
        0.001,
        0.005,
        0.01,
        0.025,
        0.05,
        0.1,
        0.25,
        0.5,
        1.0,
        2.0,
        5.0,
        15.0,
        30.0,
        60.0,
        120.0,
        300.0,
    ),
)


def _utc(dt: datetime) -> datetime:
    return dt if dt.tzinfo else dt.replace(tzinfo=UTC)


def event_lag_seconds(
    *,
    data_timestamp: datetime | None,
    feed_last_message_at: datetime | None,
    now: datetime | None = None,
) -> float | None:
    """Seconds between now and the freshest of feed time / bar data time (event lag)."""
    n = now or datetime.now(UTC)
    n = _utc(n)
    ages: list[float] = []
    if feed_last_message_at is not None:
        flm = _utc(feed_last_message_at).astimezone(UTC)
        ages.append(abs((n - flm).total_seconds()))
    if data_timestamp is not None:
        dt = _utc(data_timestamp).astimezone(UTC)
        ages.append(abs((n - dt).total_seconds()))
    if not ages:
        return None
    return max(ages)


def record_lag_seconds(
    symbol: str,
    *,
    data_ingestion_seconds: float | None,
    decision_processing_seconds: float | None,
    execution_feedback_seconds: float | None,
) -> None:
    """Observe lag histograms when values are present."""
    sym = str(symbol)
    if data_ingestion_seconds is not None:
        try:
            CANONICAL_LAG_SECONDS.labels(symbol=sym, domain="data_ingestion").observe(
                float(data_ingestion_seconds)
            )
        except (TypeError, ValueError):
            pass
    if decision_processing_seconds is not None:
        try:
            CANONICAL_LAG_SECONDS.labels(symbol=sym, domain="decision").observe(
                float(decision_processing_seconds)
            )
        except (TypeError, ValueError):
            pass
    if execution_feedback_seconds is not None:
        try:
            CANONICAL_LAG_SECONDS.labels(symbol=sym, domain="execution_feedback").observe(
                float(execution_feedback_seconds)
            )
        except (TypeError, ValueError):
            pass


def execution_feedback_lag_seconds(bucket: dict[str, Any] | None) -> float | None:
    """EMA fill latency from memory bucket → seconds for metrics."""
    if not bucket:
        return None
    lat = bucket.get("latency_ms_ema")
    if lat is None:
        return None
    try:
        return max(0.0, float(lat) / 1000.0)
    except (TypeError, ValueError):
        return None
