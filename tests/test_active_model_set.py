"""FB-SPEC-06: optional JSON manifest for promoted serving paths."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from app.config.settings import AppSettings
from models.registry.active_set import active_model_set_status, apply_active_model_set


def test_apply_manifest_overrides_paths(tmp_path: Path) -> None:
    fw = tmp_path / "promoted_f.npz"
    fw.write_bytes(b"x")
    manifest = tmp_path / "active.json"
    manifest.write_text(
        json.dumps(
            {
                "label": "nightly-2026-04-11",
                "version": 3,
                "forecaster_checkpoint_id": "chk-from-manifest",
                "forecaster_weights_path": str(fw),
                "forecaster_conformal_state_path": None,
            }
        ),
        encoding="utf-8",
    )
    base = AppSettings(
        models_forecaster_checkpoint_id="from-env",
        models_forecaster_weights_path="/wrong/path.npz",
        models_active_set_path=str(manifest),
    )
    merged = apply_active_model_set(base)
    assert merged.models_forecaster_checkpoint_id == "chk-from-manifest"
    assert merged.models_forecaster_weights_path == str(fw)
    assert merged.models_forecaster_conformal_state_path is None
    assert merged.models_active_set_label == "nightly-2026-04-11"
    assert merged.models_active_set_manifest_version == 3


def test_missing_manifest_file_leaves_settings(tmp_path: Path) -> None:
    base = AppSettings(
        models_forecaster_weights_path="/keep.npz",
        models_active_set_path=str(tmp_path / "nope.json"),
    )
    merged = apply_active_model_set(base)
    assert merged.models_forecaster_weights_path == "/keep.npz"


def test_status_reflects_manifest(tmp_path: Path) -> None:
    m = tmp_path / "a.json"
    m.write_text("{}", encoding="utf-8")
    s = AppSettings(
        models_active_set_path=str(m),
        models_active_set_label="L1",
        models_active_set_manifest_version=1,
    )
    st = active_model_set_status(s)
    assert st["manifest_file_exists"] is True
    assert st["label"] == "L1"
    assert st["manifest_version"] == 1


def test_invalid_json_logs_and_skips(monkeypatch: pytest.MonkeyPatch, tmp_path: Path, caplog: pytest.LogCaptureFixture) -> None:
    bad = tmp_path / "bad.json"
    bad.write_text("{", encoding="utf-8")
    base = AppSettings(
        models_forecaster_weights_path="/orig.npz",
        models_active_set_path=str(bad),
    )
    with caplog.at_level("ERROR"):
        merged = apply_active_model_set(base)
    assert merged.models_forecaster_weights_path == "/orig.npz"
    assert "invalid JSON" in caplog.text
