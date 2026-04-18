"""FB-CAN-022: top-level YAML ``risk:`` block must not load."""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

from app.config import settings as settings_mod


def test_legacy_risk_yaml_raises(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    yml = tmp_path / "bad.yaml"
    yml.write_text(
        yaml.safe_dump({"risk": {"max_total_exposure_usd": 1}}),
        encoding="utf-8",
    )
    monkeypatch.setattr(settings_mod, "_DEFAULT_YAML", yml)
    with pytest.raises(ValueError, match="top-level 'risk:'"):
        settings_mod.load_settings()
