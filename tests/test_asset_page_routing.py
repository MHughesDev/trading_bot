"""FB-AP-027: Asset page symbol helpers."""

from __future__ import annotations

from control_plane.asset_page_helpers import normalize_symbol, validate_symbol_display


def test_normalize_symbol() -> None:
    assert normalize_symbol("  btc-usd  ") == "BTC-USD"


def test_validate_symbol_display() -> None:
    assert validate_symbol_display("BTC-USD") is True
    assert validate_symbol_display("") is False
    assert validate_symbol_display("BTC/USD") is False
