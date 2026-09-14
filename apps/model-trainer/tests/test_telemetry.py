"""The bounded telemetry queue (AT-55)."""
from __future__ import annotations
import pathlib, sys
import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
from app.telemetry import BoundedTelemetry  # noqa: E402


def test_the_queue_drops_the_oldest_not_the_newest():
    """Dropping the newest keeps a stale view that never catches up — worse than
    useless on a progress display, because it looks live."""
    q = BoundedTelemetry(capacity=3)
    for i in range(6):
        q.publish(i)
    assert q.drain() == [3, 4, 5], "the present survives, the past is dropped"
    assert q.dropped == 3


def test_publishing_never_blocks_or_raises():
    q = BoundedTelemetry(capacity=1)
    for i in range(10_000):
        q.publish(i)  # a training loop must not be slowed by a display
    assert q.drain() == [9_999]
    assert q.dropped == 9_999


def test_the_drop_count_travels_with_the_messages():
    """A gap the consumer cannot see is a gap it reads as quiet."""
    q = BoundedTelemetry(capacity=2)
    for i in range(5):
        q.publish(i)
    snap = q.snapshot()
    assert snap == {"queued": 2, "dropped": 3, "capacity": 2}
    assert q.is_lagging()


def test_draining_does_not_reset_the_drop_counter():
    """Resetting on read would hide a consumer that is chronically behind."""
    q = BoundedTelemetry(capacity=1)
    q.publish("a"); q.publish("b")
    q.drain()
    assert q.dropped == 1
    q.drain()
    assert q.dropped == 1


def test_a_healthy_queue_is_not_lagging():
    q = BoundedTelemetry(capacity=8)
    for i in range(8):
        q.publish(i)
    assert not q.is_lagging()
    assert q.dropped == 0


def test_a_zero_capacity_queue_is_a_caller_mistake():
    with pytest.raises(ValueError, match="at least one"):
        BoundedTelemetry(capacity=0)
