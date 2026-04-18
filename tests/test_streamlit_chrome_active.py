"""FB-UX-018: sidebar helper behavior."""

from __future__ import annotations

from control_plane.streamlit_chrome import (
    _active_watching_symbols,
    _position_rows_from_payload,
    _watching_suffix_from_status,
)


def test_active_watching_symbols_filters_and_sorts() -> None:
    st_data = {
        "asset_lifecycle": {
            "states": {
                "SOL-USD": "active",
                "BTC-USD": "initialized_not_active",
                "ETH-USD": "active",
            }
        }
    }
    assert _active_watching_symbols(st_data) == ["ETH-USD", "SOL-USD"]


def test_active_watching_symbols_empty() -> None:
    assert _active_watching_symbols({}) == []
    assert _active_watching_symbols({"asset_lifecycle": {}}) == []


def test_watching_suffix_from_status_uses_cache_when_available() -> None:
    st_data = {"price_cache": {"BTC-USD": {"last_price": "65000.1", "delta_24h_pct": "2.5"}}}
    out = _watching_suffix_from_status(st_data, "BTC-USD")
    assert "65000.1" in out
    assert "+2.50" in out


def test_position_rows_filters_nonzero() -> None:
    payload = {
        "positions": [
            {"symbol": "BTC-USD", "quantity": "0.1", "unrealized_pnl": "12.4"},
            {"symbol": "ETH-USD", "quantity": "0"},
            {"symbol": "SOL-USD", "quantity": "-1"},
        ]
    }
    out = _position_rows_from_payload(payload)
    assert [r["symbol"] for r in out] == ["BTC-USD", "SOL-USD"]
