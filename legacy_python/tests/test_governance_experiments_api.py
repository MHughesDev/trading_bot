"""FB-CAN-027: experiment registry HTTP API."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

import control_plane.api as api
from orchestration.release_gating import ReleaseCandidate, ReleaseLedger, write_release_ledger


@pytest.fixture
def client_with_registry(tmp_path, monkeypatch: pytest.MonkeyPatch) -> TestClient:
    reg_path = tmp_path / "experiment_registry.json"
    ledger_path = tmp_path / "release_ledger.json"
    led = ReleaseLedger(
        candidates=[
            ReleaseCandidate(
                release_id="rel-ci",
                kind="config",
                owner="ops",
                config_version="1.0.0",
            )
        ]
    )
    write_release_ledger(led, ledger_path)

    import models.registry.experiment_registry as er_mod

    monkeypatch.setattr(er_mod, "default_experiment_registry_path", lambda: reg_path)
    monkeypatch.setattr(er_mod, "default_release_ledger_path", lambda: ledger_path)

    return TestClient(api.app)


def test_list_experiments_empty(client_with_registry: TestClient) -> None:
    r = client_with_registry.get("/governance/experiments")
    assert r.status_code == 200
    assert r.json() == {"experiments": [], "count": 0}


def test_post_get_delete_roundtrip(client_with_registry: TestClient) -> None:
    body = {
        "experiment_id": "exp-test-1",
        "title": "Test experiment",
        "owner": "alice",
        "hypothesis": "higher threshold reduces false positives",
        "metrics_defined_before_run": ["false_positive_rate"],
        "status": "draft",
        "linked_release_candidate": "rel-ci",
    }
    r = client_with_registry.post("/governance/experiments", json=body)
    assert r.status_code == 200

    g = client_with_registry.get("/governance/experiments/exp-test-1")
    assert g.status_code == 200
    assert g.json()["experiment_id"] == "exp-test-1"

    lst = client_with_registry.get("/governance/experiments?linked_release=rel-ci")
    assert lst.json()["count"] == 1

    d = client_with_registry.delete("/governance/experiments/exp-test-1")
    assert d.status_code == 200


def test_post_invalid_transition(client_with_registry: TestClient) -> None:
    client_with_registry.post(
        "/governance/experiments",
        json={
            "experiment_id": "exp-bad",
            "title": "t",
            "owner": "o",
            "hypothesis": "h",
            "metrics_defined_before_run": ["m"],
            "status": "draft",
        },
    )
    r = client_with_registry.post(
        "/governance/experiments",
        json={
            "experiment_id": "exp-bad",
            "title": "t",
            "owner": "o",
            "hypothesis": "h",
            "metrics_defined_before_run": ["m"],
            "status": "candidate_for_release",
        },
    )
    assert r.status_code == 422
