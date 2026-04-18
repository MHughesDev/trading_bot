"""FB-UX-020 dashboard panel data shaping."""

from __future__ import annotations

from control_plane.pnl_panel import _active_watching_rows, _holdings_rows


def test_active_watching_rows_empty() -> None:
    assert _active_watching_rows({}) == []


def test_active_watching_rows_populated_with_cache() -> None:
    payload = {
        "asset_lifecycle": {"states": {"BTC-USD": "active", "ETH-USD": "initialized_not_active"}},
        "price_cache": {"BTC-USD": {"last_price": "64000", "delta_24h_pct": "1.25"}},
    }
    out = _active_watching_rows(payload)
    assert out == [{"symbol": "BTC-USD", "last_price": "64000", "delta_24h_pct": "1.25"}]


def test_holdings_rows_filters_zero_qty_and_computes_market_value() -> None:
    payload = {
        "positions": [
            {"symbol": "BTC-USD", "quantity": "0.1", "avg_entry_price": "50000", "mark_price": "60000", "unrealized_pnl": "1000"},
            {"symbol": "ETH-USD", "quantity": "0", "mark_price": "2000", "unrealized_pnl": "0"},
        ]
    }
    out = _holdings_rows(payload)
    assert len(out) == 1
    assert out[0]["symbol"] == "BTC-USD"
    assert out[0]["market_value"] == "6000.0"
