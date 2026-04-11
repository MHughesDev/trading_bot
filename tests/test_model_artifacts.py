"""FB-SPEC-03: unified model artifact contract."""

from __future__ import annotations

from pathlib import Path

import pytest
from pydantic import SecretStr

from app.config.model_artifacts import TRAINING_QUANTILE_FORECASTER_JOBLIB, model_artifact_contract
from app.config.settings import AppSettings
from control_plane import api
from fastapi.testclient import TestClient


def test_contract_training_disclaimer_names_joblib() -> None:
    c = model_artifact_contract(AppSettings())
    assert "active_model_set" in c
    assert TRAINING_QUANTILE_FORECASTER_JOBLIB in c["training"]["note"]
    assert c["serving"]["forecaster_forward"] == "numpy_rng"
    assert c["serving"]["policy_actor"] == "heuristic"


def test_contract_file_exists_when_paths_present(tmp_path: Path) -> None:
    fw = tmp_path / "f.npz"
    fw.write_bytes(b"fake")
    pp = tmp_path / "p.npz"
    pp.write_bytes(b"fake")
    conf = tmp_path / "c.json"
    conf.write_text("{}")
    s = AppSettings(
        models_forecaster_weights_path=str(fw),
        models_policy_mlp_path=str(pp),
        models_forecaster_conformal_state_path=str(conf),
        models_forecaster_checkpoint_id="cid-1",
    )
    c = model_artifact_contract(s)
    assert c["serving"]["forecaster_weights_file_exists"] is True
    assert c["serving"]["policy_mlp_file_exists"] is True
    assert c["serving"]["conformal_state_file_exists"] is True
    assert c["serving"]["forecaster_forward"] == "npz_weights"
    assert c["serving"]["policy_actor"] == "mlp_npz"
    assert c["serving"]["lineage_checkpoint_id"] == "cid-1"


def test_status_includes_model_artifacts(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            execution_mode="paper",
            risk_signing_secret=SecretStr("x" * 32),
            allow_unsigned_execution=False,
            alpaca_api_key=SecretStr("k"),
            alpaca_api_secret=SecretStr("s"),
        ),
    )
    client = TestClient(api.app)
    r = client.get("/status")
    assert r.status_code == 200
    body = r.json()
    assert "model_artifacts" in body
    ma = body["model_artifacts"]
    assert "serving" in ma and "training" in ma and "registry" in ma


def test_post_models_version_requires_key_when_configured(monkeypatch: pytest.MonkeyPatch) -> None:
    """FB-SPEC-06: mutating label endpoint respects control plane API key."""
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="cp-secret"))
    client = TestClient(api.app)
    r = client.post("/models/version", json={"component": "forecaster", "version": "v9"})
    assert r.status_code == 401


def test_post_models_version_sets_prometheus_gauge(monkeypatch: pytest.MonkeyPatch) -> None:
    """FB-SPEC-06: operator labels flow to nm_model_version_info."""
    from prometheus_client import REGISTRY, generate_latest

    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="cp-secret"))
    client = TestClient(api.app)
    r = client.post(
        "/models/version",
        json={"component": "policy", "version": "build-42"},
        headers={"X-API-Key": "cp-secret"},
    )
    assert r.status_code == 200
    assert r.json() == {"component": "policy", "version": "build-42"}

    payload = generate_latest(REGISTRY).decode()
    assert 'nm_model_version_info{component="policy",version="build-42"} 1.0' in payload
