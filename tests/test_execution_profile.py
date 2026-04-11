"""Execution profile persistence and config patching."""

from __future__ import annotations

from pathlib import Path

import yaml

from control_plane import execution_profile as ep


def test_write_pending_when_differs_then_clear_when_matches(tmp_path: Path, monkeypatch):
    monkeypatch.setattr("control_plane.execution_profile._STATE_PATH", tmp_path / "execution_profile.json")
    ep.write_pending_intent("live", "paper")
    assert ep.read_pending_intent() == "live"
    assert ep.profile_payload("paper")["restart_required"] is True
    ep.write_pending_intent("paper", "paper")
    assert ep.read_pending_intent() is None
    assert ep.profile_payload("paper")["restart_required"] is False


def test_patch_default_yaml_and_dot_env(tmp_path: Path, monkeypatch):
    dy = tmp_path / "default.yaml"
    dy.write_text(
        yaml.safe_dump({"execution": {"mode": "paper", "live_adapter": "coinbase"}}),
        encoding="utf-8",
    )
    envf = tmp_path / ".env"
    envf.write_text("NM_EXECUTION_MODE=paper\n", encoding="utf-8")
    monkeypatch.setattr("control_plane.execution_profile._DEFAULT_YAML", dy)
    monkeypatch.setattr("control_plane.execution_profile._REPO_ROOT", tmp_path)

    out = ep.apply_intent_to_config_files("live")
    assert out["default_yaml"] is True
    assert out["dot_env"] is True
    loaded = yaml.safe_load(dy.read_text(encoding="utf-8"))
    assert loaded["execution"]["mode"] == "live"
    assert "NM_EXECUTION_MODE=live" in envf.read_text(encoding="utf-8")
