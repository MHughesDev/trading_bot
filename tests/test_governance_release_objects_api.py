"""FB-CAN-051: release-object HTTP API."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

import control_plane.api as api
from app.contracts.release_objects import ReleaseCandidate, ReleaseLedger, write_release_ledger
from orchestration.fault_injection_profiles import list_canonical_fault_profile_ids
from orchestration.release_gating import EvidencePackage, RollbackTarget


@pytest.fixture
def client_with_ledger(tmp_path, monkeypatch: pytest.MonkeyPatch) -> TestClient:
    ledger_path = tmp_path / "release_ledger.json"
    led = ReleaseLedger(
        candidates=[
            ReleaseCandidate(
                release_id="rel-seed",
                kind="config",
                owner="ops",
                config_version="1.0.0",
                evidence=EvidencePackage(version_identifiers={"config": "1.0.0"}),
                rollback=RollbackTarget(target_config_version="0.9.0"),
            )
        ]
    )
    write_release_ledger(led, ledger_path)

    import app.contracts.release_objects as ro

    monkeypatch.setattr(ro, "default_release_ledger_path", lambda: ledger_path)

    return TestClient(api.app)


def test_list_get_release_objects(client_with_ledger: TestClient) -> None:
    r = client_with_ledger.get("/governance/release-objects")
    assert r.status_code == 200
    js = r.json()
    assert js["count"] == 1

    g = client_with_ledger.get("/governance/release-objects/rel-seed")
    assert g.status_code == 200
    assert g.json()["release_id"] == "rel-seed"


def test_evaluate_gates(client_with_ledger: TestClient) -> None:
    fault_ids = list(list_canonical_fault_profile_ids())
    body = {
        "candidate": {
            "release_id": "rel-eval",
            "kind": "config",
            "owner": "ops",
            "config_version": "1.0.0",
            "evidence": {
                "version_identifiers": {"config": "1.0.0"},
                "replay_summary": "ok",
                "replay_run_ids": ["r1"],
                "scenario_stress_summary": "ok",
                "known_risks": "none",
                "owner_approval_present": True,
                "shadow_divergence_reviewed": True,
                "shadow_comparison_passed": True,
                "fault_stress_run_ids": ["ci-fault-suite"],
                "fault_profile_ids_satisfied": fault_ids,
            },
            "rollback": {"target_config_version": "0.9.0"},
        },
        "target_environment": "live",
    }
    r = client_with_ledger.post("/governance/release-objects/evaluate-gates", json=body)
    assert r.status_code == 200
    assert r.json()["allowed"] is True
