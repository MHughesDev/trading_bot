"""execution.adapter_registry (FB-N1 visibility)."""

from __future__ import annotations

from app.config.settings import AppSettings
from execution.adapter_registry import (
    KNOWN_EXECUTION_ADAPTER_OVERRIDES,
    SUPPORTED_LIVE_ADAPTERS,
    SUPPORTED_PAPER_ADAPTERS,
    supported_adapters_for_settings,
)


def test_supported_sets_non_empty() -> None:
    assert "alpaca" in SUPPORTED_PAPER_ADAPTERS
    assert "coinbase" in SUPPORTED_LIVE_ADAPTERS
    assert "stub" in KNOWN_EXECUTION_ADAPTER_OVERRIDES


def test_supported_adapters_payload() -> None:
    s = AppSettings()
    p = supported_adapters_for_settings(s)
    assert p["paper_adapter_configured"] == "alpaca"
    assert p["live_adapter_configured"] == "coinbase"
    assert p["paper_supported"] is True
    assert p["live_supported"] is True
    assert "note" in p
