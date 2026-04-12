"""FB-AP-033: active assets in sidebar chrome."""

from __future__ import annotations

from control_plane.streamlit_chrome import _active_watching_symbols


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
