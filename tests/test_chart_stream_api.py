"""FB-AP-034: GET /assets/chart/latest-bar and /assets/chart/stream (SSE)."""

from __future__ import annotations

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


def test_latest_bar_404(client_no_auth, monkeypatch):
    async def fake_latest(*_a, **_k):
        return None

    monkeypatch.setattr(api, "query_latest_canonical_bar_for_chart", fake_latest)
    r = client_no_auth.get(
        "/assets/chart/latest-bar",
        params={"symbol": "BTC-USD", "interval_seconds": 60},
    )
    assert r.status_code == 404


def test_latest_bar_ok(client_no_auth, monkeypatch):
    async def fake_latest(*_a, **_k):
        return {
            "ts": "2026-04-01T00:00:00+00:00",
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

    monkeypatch.setattr(api, "query_latest_canonical_bar_for_chart", fake_latest)
    r = client_no_auth.get(
        "/assets/chart/latest-bar",
        params={"symbol": "BTC-USD", "interval_seconds": 60},
    )
    assert r.status_code == 200
    j = r.json()
    assert j["ok"] is True
    assert j["bar"]["close"] == 1.5


def test_sse_stream_headers_and_first_chunk(client_no_auth, monkeypatch):
    async def fake_sse(_settings, *, symbol, interval_seconds, poll_seconds):
        yield 'data: {"type":"hello"}\n\n'

    monkeypatch.setattr(api, "sse_chart_bar_updates", fake_sse)
    with client_no_auth.stream(
        "GET",
        "/assets/chart/stream",
        params={"symbol": "BTC-USD", "interval_seconds": 60, "poll_seconds": 1.0},
    ) as r:
        assert r.status_code == 200
        assert "text/event-stream" in r.headers.get("content-type", "")
        body = r.read()
        assert b"hello" in body
