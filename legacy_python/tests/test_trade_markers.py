"""FB-AP-025: trade markers ledger and API."""

from __future__ import annotations

from datetime import UTC, datetime

from execution.trade_markers import (
    TradeMarker,
    append_marker,
    iter_markers,
    markers_path,
)


def test_append_and_query_symbol_window(tmp_path) -> None:
    p = tmp_path / "m.jsonl"
    t0 = datetime(2026, 4, 12, 10, 0, 0, tzinfo=UTC)
    append_marker(
        TradeMarker(
            ts=t0,
            symbol="BTC-USD",
            side="buy",
            quantity="0.01",
            source="intent_submit",
            correlation_id="c1",
            execution_mode="paper",
        ),
        path=p,
    )
    t1 = datetime(2026, 4, 12, 11, 0, 0, tzinfo=UTC)
    append_marker(
        TradeMarker(
            ts=t1,
            symbol="ETH-USD",
            side="sell",
            quantity="1",
            source="intent_submit",
            correlation_id="c2",
            execution_mode="paper",
        ),
        path=p,
    )
    out = iter_markers(
        symbol="BTC-USD",
        start=datetime(2026, 4, 12, 9, 0, 0, tzinfo=UTC),
        end=datetime(2026, 4, 12, 12, 0, 0, tzinfo=UTC),
        path=p,
    )
    assert len(out) == 1
    assert out[0].symbol == "BTC-USD"
    assert out[0].side == "buy"


def test_markers_path_under_repo() -> None:
    assert markers_path().name == "trade_markers.jsonl"
