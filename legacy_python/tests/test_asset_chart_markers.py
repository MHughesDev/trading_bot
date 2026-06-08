"""FB-AP-029: trade marker overlay y-alignment."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta

from control_plane.asset_chart_markers import trade_marker_buy_sell_traces


def test_buy_sell_y_matches_bar_close() -> None:
    t0 = datetime(2026, 4, 1, 12, 0, 0, tzinfo=UTC)
    bars = [
        {
            "ts": t0.isoformat(),
            "open": 100.0,
            "high": 101.0,
            "low": 99.0,
            "close": 100.5,
            "volume": 1.0,
        },
        {
            "ts": (t0 + timedelta(hours=1)).isoformat(),
            "open": 100.5,
            "high": 102.0,
            "low": 100.0,
            "close": 101.0,
            "volume": 2.0,
        },
    ]
    markers = [
        {
            "ts": (t0 + timedelta(minutes=30)).isoformat(),
            "side": "buy",
            "quantity": "0.1",
            "source": "intent_submit",
        },
        {
            "ts": (t0 + timedelta(hours=1, minutes=15)).isoformat(),
            "side": "sell",
            "quantity": "0.1",
            "source": "intent_submit",
        },
    ]
    buy_tr, sell_tr = trade_marker_buy_sell_traces(markers, bars)
    assert len(buy_tr["y"]) == 1
    assert buy_tr["y"][0] == 100.5
    assert len(sell_tr["y"]) == 1
    assert sell_tr["y"][0] == 101.0
