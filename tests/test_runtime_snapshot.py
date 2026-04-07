from __future__ import annotations

from pathlib import Path

from app.config.settings import load_settings
from app.contracts.common import SystemMode
from app.runtime.runtime_service import NautilusRuntimeService


def test_runtime_snapshot_has_required_top_level_fields() -> None:
    settings = load_settings(Path(__file__).resolve().parents[1] / "app/config/settings.yaml")
    runtime = NautilusRuntimeService(settings)
    snapshot = runtime.get_snapshot()

    assert "system_mode" in snapshot
    assert "execution_mode" in snapshot
    assert "portfolio" in snapshot
    assert "symbols" in snapshot
    assert "stats" in snapshot


def test_runtime_mode_update_reflects_in_snapshot() -> None:
    settings = load_settings(Path(__file__).resolve().parents[1] / "app/config/settings.yaml")
    runtime = NautilusRuntimeService(settings)
    runtime.set_system_mode(SystemMode.PAUSE_NEW_ENTRIES)
    snapshot = runtime.get_snapshot()
    assert snapshot["system_mode"] == SystemMode.PAUSE_NEW_ENTRIES.value
