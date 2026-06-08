"""FB-AP-024: GET /assets/chart/bars — symbol-scoped canonical bars."""

from __future__ import annotations

from datetime import UTC, datetime
import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_model_registry as reg
from control_plane import api


@pytest.fixture
def client_no_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_chart_bars_requires_symbol(client_no_auth):
    r = client_no_auth.get(
        "/assets/chart/bars",
        params={
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 422


def test_chart_bars_rejects_start_after_end(client_no_auth):
    r = client_no_auth.get(
        "/assets/chart/bars",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-02T00:00:00Z",
            "end": "2026-04-01T00:00:00Z",
        },
    )
    assert r.status_code == 422


@pytest.mark.asyncio
async def test_query_canonical_bars_for_chart_maps_rows(monkeypatch):
    from control_plane.chart_bars import query_canonical_bars_for_chart

    settings = AppSettings()
    fake_rows = [
        {
            "ts": datetime(2026, 4, 1, 0, 0, 0, tzinfo=UTC),
            "symbol": "BTC-USD",
            "interval_seconds": 60,
            "open": 1.0,
            "high": 2.0,
            "low": 0.5,
            "close": 1.5,
            "volume": 10.0,
            "source": "kraken",
            "schema_version": 1,
        }
    ]

    class FakeQdb:
        async def connect(self):
            return None

        async def aclose(self):
            return None

        async def query_bars(self, *args, **kwargs):
            return list(fake_rows)

    monkeypatch.setattr(
        "control_plane.chart_bars.QuestDBWriter",
        lambda *a, **k: FakeQdb(),
    )
    out = await query_canonical_bars_for_chart(
        settings,
        symbol="BTC-USD",
        start=datetime(2026, 3, 31, 0, 0, 0, tzinfo=UTC),
        end=datetime(2026, 4, 3, 0, 0, 0, tzinfo=UTC),
        interval_seconds=60,
        limit=100,
    )
    assert out["count"] == 1
    assert out["bars"][0]["close"] == 1.5
    assert "2026-04-01" in out["bars"][0]["ts"]


def test_chart_bars_endpoint_ok(client_no_auth, monkeypatch):
    async def fake_query(*_a, **_k):
        return {
            "symbol": "BTC-USD",
            "interval_seconds": 60,
            "start": "2026-04-01T00:00:00+00:00",
            "end": "2026-04-02T00:00:00+00:00",
            "limit": 5000,
            "count": 0,
            "bars": [],
        }

    monkeypatch.setattr(api, "query_canonical_bars_for_chart", fake_query)
    r = client_no_auth.get(
        "/assets/chart/bars",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 200
    assert r.json()["symbol"] == "BTC-USD"
    assert r.json()["bars"] == []
