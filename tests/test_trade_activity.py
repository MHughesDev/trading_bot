"""FB-AP-037 trade activity helper."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from pathlib import Path

from execution.trade_activity import symbol_had_trade_in_lookback
from execution.trade_markers import TradeMarker, append_marker


def test_symbol_had_trade_in_lookback_true(tmp_path: Path) -> None:
    p = tmp_path / "trade_markers.jsonl"
    t0 = datetime(2026, 4, 10, 12, 0, 0, tzinfo=UTC)
    append_marker(
        TradeMarker(
            ts=t0,
            symbol="BTC-USD",
            side="buy",
            quantity="1",
            source="intent_submit",
        ),
        path=p,
    )
    assert symbol_had_trade_in_lookback(
        "BTC-USD",
        7,
        now=t0 + timedelta(days=1),
        markers_file=p,
    )


def test_symbol_had_trade_in_lookback_false(tmp_path: Path) -> None:
    p = tmp_path / "trade_markers.jsonl"
    t0 = datetime(2026, 4, 1, 12, 0, 0, tzinfo=UTC)
    append_marker(
        TradeMarker(
            ts=t0,
            symbol="BTC-USD",
            side="buy",
            quantity="1",
            source="intent_submit",
        ),
        path=p,
    )
    assert not symbol_had_trade_in_lookback(
        "BTC-USD",
        7,
        now=datetime(2026, 4, 15, 12, 0, 0, tzinfo=UTC),
        markers_file=p,
    )
