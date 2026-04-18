"""FB-CAN-026: governance release evidence HTTP endpoints."""

from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import control_plane.api as api


def test_get_release_evidence() -> None:
    c = TestClient(api.app)
    r = c.get("/governance/release-evidence")
    assert r.status_code == 200
    data = r.json()
    assert "canonical_config_fingerprint" in data
    assert "config_version" in data


def test_post_release_evidence_diff(tmp_path) -> None:
    default_yaml = Path(__file__).resolve().parents[1] / "app" / "config" / "default.yaml"
    text = default_yaml.read_text(encoding="utf-8")
    c = TestClient(api.app)
    r = c.post("/governance/release-evidence/diff", json={"baseline_yaml": text})
    assert r.status_code == 200
    d = r.json()
    assert "change_count" in d
    assert "changes" not in d or isinstance(d.get("changes"), list)


def test_post_release_evidence_diff_rejects_empty() -> None:
    c = TestClient(api.app)
    r = c.post("/governance/release-evidence/diff", json={"baseline_yaml": ""})
    assert r.status_code == 422
