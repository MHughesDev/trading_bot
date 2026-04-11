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
