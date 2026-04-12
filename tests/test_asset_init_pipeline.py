"""FB-AP-006: asset init pipeline orchestrator."""

from __future__ import annotations

import time

import polars as pl
import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api
from orchestration import asset_init_pipeline as pipe


@pytest.fixture(autouse=True)
def reset_pipeline():
    pipe.reset_asset_init_pipeline_for_tests()
    yield
    pipe.reset_asset_init_pipeline_for_tests()


@pytest.fixture
def client_no_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_post_init_starts_job(client_no_auth: TestClient, monkeypatch) -> None:
    def fake_fetch(symbol, start, end, granularity_seconds=60):
        return pl.DataFrame(
            {
                "timestamp": [start],
                "open": [1.0],
                "high": [1.0],
                "low": [1.0],
                "close": [1.0],
                "volume": [1.0],
            }
        )

    monkeypatch.setattr(
        "orchestration.real_data_bars.fetch_symbol_bars_sync",
        fake_fetch,
    )

    r = client_no_auth.post("/assets/init/BTC-USD")
    assert r.status_code == 200
    job_id = r.json()["job_id"]

    for _ in range(100):
        j = client_no_auth.get(f"/assets/init/jobs/{job_id}").json()
        if j["status"] in ("succeeded", "failed"):
            break
        time.sleep(0.05)
    assert j["status"] == "succeeded"
    steps = [s["step"] for s in j["steps"]]
    assert steps[0] == "kraken_fetch"
    assert "validate" in steps


def test_concurrent_init_conflict(client_no_auth: TestClient, monkeypatch) -> None:
    import threading

    ev = threading.Event()
    started = threading.Event()

    def slow_fetch(symbol, start, end, granularity_seconds=60):
        started.set()
        ev.wait(timeout=30)
        return pl.DataFrame(
            {
                "timestamp": [start],
                "open": [1.0],
                "high": [1.0],
                "low": [1.0],
                "close": [1.0],
                "volume": [1.0],
            }
        )

    monkeypatch.setattr("orchestration.real_data_bars.fetch_symbol_bars_sync", slow_fetch)

    r1 = client_no_auth.post("/assets/init/AAA-USD")
    assert r1.status_code == 200
    assert started.wait(timeout=5.0)
    r2 = client_no_auth.post("/assets/init/BBB-USD")
    assert r2.status_code == 409
    ev.set()


def test_get_unknown_job_404(client_no_auth: TestClient) -> None:
    assert client_no_auth.get("/assets/init/jobs/not-a-uuid").status_code == 404
