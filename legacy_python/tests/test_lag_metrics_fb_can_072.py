"""FB-CAN-072: per-domain lag decomposition helpers."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

import pytest

from observability.lag_metrics import (
    event_lag_seconds,
    execution_feedback_lag_seconds,
    record_lag_seconds,
)


def test_event_lag_seconds_max_of_feed_and_data_ts():
    now = datetime(2026, 4, 18, 12, 0, 0, tzinfo=UTC)
    feed = now - timedelta(seconds=10)
    bar = now - timedelta(seconds=50)
    lag = event_lag_seconds(
        data_timestamp=bar,
        feed_last_message_at=feed,
        now=now,
    )
    assert lag == pytest.approx(50.0)


def test_event_lag_seconds_feed_only():
    now = datetime(2026, 4, 18, 12, 0, 0, tzinfo=UTC)
    feed = now - timedelta(seconds=3)
    lag = event_lag_seconds(data_timestamp=None, feed_last_message_at=feed, now=now)
    assert lag == pytest.approx(3.0)


def test_execution_feedback_lag_seconds_from_bucket():
    assert execution_feedback_lag_seconds({"latency_ms_ema": 500.0}) == pytest.approx(0.5)
    assert execution_feedback_lag_seconds({}) is None


def test_record_lag_seconds_no_raise():
    record_lag_seconds(
        "BTC-USD",
        data_ingestion_seconds=1.0,
        decision_processing_seconds=0.05,
        execution_feedback_seconds=0.1,
    )
