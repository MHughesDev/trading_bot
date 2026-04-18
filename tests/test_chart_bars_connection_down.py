from __future__ import annotations

import psycopg
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_model_registry as reg
from control_plane import api


def test_chart_bars_operational_error_returns_503(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))

    async def _raise_conn(*_a, **_k):
        raise psycopg.OperationalError("connection refused")

    monkeypatch.setattr(api, "query_canonical_bars_for_chart", _raise_conn)
    client = TestClient(api.app)
    r = client.get(
        "/assets/chart/bars",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 503
    assert r.json()["error"] == "storage_unavailable"
    assert "connection refused" in r.json()["detail"]
