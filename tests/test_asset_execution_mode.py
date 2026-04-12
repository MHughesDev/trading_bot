"""Per-symbol execution mode sidecar (FB-AP-030)."""

from __future__ import annotations

import pytest

from app.config.settings import AppSettings
from app.runtime import asset_execution_mode as em


@pytest.fixture
def mode_tmp(tmp_path, monkeypatch):
    monkeypatch.setattr(em, "_DEFAULT_DIR", tmp_path / "exec_mode")


def test_effective_falls_back_to_settings(mode_tmp) -> None:
    s = AppSettings(execution_mode="paper")
    assert em.effective_execution_mode("BTC-USD", s) == "paper"
    s2 = AppSettings(execution_mode="live")
    assert em.effective_execution_mode("BTC-USD", s2) == "live"


def test_write_read_override(mode_tmp) -> None:
    s = AppSettings(execution_mode="paper")
    em.write_mode_override("BTC-USD", "live")
    assert em.read_mode_override("BTC-USD") == "live"
    assert em.effective_execution_mode("BTC-USD", s) == "live"


def test_delete_clears_override(mode_tmp) -> None:
    s = AppSettings(execution_mode="paper")
    em.write_mode_override("ETH-USD", "live")
    assert em.delete_mode_override("ETH-USD")
    assert em.read_mode_override("ETH-USD") is None
    assert em.effective_execution_mode("ETH-USD", s) == "paper"


def test_list_overrides(mode_tmp) -> None:
    em.write_mode_override("A", "paper")
    em.write_mode_override("B", "live")
    rows = em.list_mode_overrides()
    syms = {r["symbol"]: r["execution_mode"] for r in rows}
    assert syms == {"A": "paper", "B": "live"}


def test_to_api_dict(mode_tmp) -> None:
    s = AppSettings(execution_mode="paper")
    d = em.to_api_dict("BTC-USD", s)
    assert d["default_execution_mode"] == "paper"
    assert d["execution_mode"] == "paper"
    em.write_mode_override("BTC-USD", "live")
    d2 = em.to_api_dict("BTC-USD", s)
    assert d2["override"] == "live"
    assert d2["execution_mode"] == "live"
