"""Tests for per-asset init pipeline (FB-AP-006 / FB-AP-007)."""

from __future__ import annotations

import threading
import time

import polars as pl
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api
from orchestration.asset_init_pipeline import reset_asset_init_pipeline_for_tests


def test_post_assets_init_and_poll_status(monkeypatch) -> None:
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    reset_asset_init_pipeline_for_tests()

    def fake_fetch(*_a, **_k):
        from datetime import UTC, datetime

        t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
        return pl.DataFrame(
            {
                "timestamp": [t0],
                "open": [1.0],
                "high": [1.0],
                "low": [1.0],
                "close": [1.0],
                "volume": [1.0],
            }
        )

    monkeypatch.setattr(
        "orchestration.init_kraken_historical.fetch_symbol_bars_sync",
        fake_fetch,
    )

    client = TestClient(api.app)
    r = client.post("/assets/init/BTC-USD")
    assert r.status_code == 200
    job_id = r.json()["job_id"]

    for _ in range(50):
        st = client.get(f"/assets/init/jobs/{job_id}")
        assert st.status_code == 200
        if st.json().get("status") in ("succeeded", "failed"):
            break
        time.sleep(0.05)
    body = st.json()
    assert body["status"] == "succeeded"
    steps = body["steps"]
    assert steps[0]["step"] == "kraken_fetch"
    assert steps[0]["status"] == "done"
    assert "XBTUSD" in (steps[0].get("detail") or "")
    assert steps[1]["step"] == "validate"
    assert steps[1]["status"] == "done"
    assert "meta=" in (steps[1].get("detail") or "")
    reset_asset_init_pipeline_for_tests()


def test_second_init_returns_409_when_busy(monkeypatch) -> None:
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    reset_asset_init_pipeline_for_tests()
    hold = threading.Event()

    def slow_fetch(*_a, **_k):
        from datetime import UTC, datetime

        hold.wait(timeout=5.0)
        t0 = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
        return pl.DataFrame(
            {
                "timestamp": [t0],
                "open": [1.0],
                "high": [1.0],
                "low": [1.0],
                "close": [1.0],
                "volume": [1.0],
            }
        )

    monkeypatch.setattr(
        "orchestration.init_kraken_historical.fetch_symbol_bars_sync",
        slow_fetch,
    )

    client = TestClient(api.app)
    r1 = client.post("/assets/init/BTC-USD")
    assert r1.status_code == 200
    try:
        r2 = client.post("/assets/init/ETH-USD")
        assert r2.status_code == 409
    finally:
        hold.set()
        time.sleep(0.2)
    reset_asset_init_pipeline_for_tests()
