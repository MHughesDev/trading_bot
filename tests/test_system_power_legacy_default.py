"""FB-AP-039: global system power disabled by default."""

from __future__ import annotations

from pathlib import Path

import pytest

from app.runtime import system_power as sp


def test_legacy_off_is_always_on(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sp, "legacy_system_power_enabled", lambda: False)
    assert sp.is_on() is True
    assert sp.get_power() == "on"
    assert sp.set_power("off") == "on"


def test_legacy_on_respects_set(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setattr(sp, "legacy_system_power_enabled", lambda: True)
    monkeypatch.setattr(sp, "_STATE_PATH", tmp_path / "system_power.json")
    sp.set_power("off")
    assert sp.is_on() is False
