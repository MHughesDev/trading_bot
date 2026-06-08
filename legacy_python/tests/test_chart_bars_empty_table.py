from __future__ import annotations

import psycopg
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_model_registry as reg
from control_plane import api


def test_chart_bars_undefined_table_returns_empty_with_warning(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))

    async def _raise_undefined(*_a, **_k):
        raise psycopg.errors.UndefinedTable("canonical_bars missing")

    monkeypatch.setattr(api, "query_canonical_bars_for_chart", _raise_undefined)
    client = TestClient(api.app)
    r = client.get(
        "/assets/chart/bars",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 200
    body = r.json()
    assert body["count"] == 0
    assert body["bars"] == []
    assert "warning" in body
