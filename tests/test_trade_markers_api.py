"""FB-AP-025: GET /assets/chart/trade-markers."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_model_registry as reg
from control_plane import api
from execution.trade_markers import TradeMarker, append_marker


@pytest.fixture
def client_no_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


def test_trade_markers_empty(client_no_auth, tmp_path, monkeypatch):
    from execution import trade_markers as tm

    monkeypatch.setattr(
        tm,
        "markers_path",
        lambda repo_root=None: tmp_path / "trade_markers.jsonl",
    )
    r = client_no_auth.get(
        "/assets/chart/trade-markers",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 200
    body = r.json()
    assert body["count"] == 0
    assert body["markers"] == []


def test_trade_markers_filters_symbol(client_no_auth, tmp_path, monkeypatch):
    from execution import trade_markers as tm

    p = tmp_path / "trade_markers.jsonl"
    monkeypatch.setattr(
        tm,
        "markers_path",
        lambda repo_root=None: p,
    )
    append_marker(
        TradeMarker(
            ts=datetime(2026, 4, 1, 12, 0, 0, tzinfo=UTC),
            symbol="BTC-USD",
            side="buy",
            quantity="0.1",
            source="intent_submit",
            correlation_id="x",
            execution_mode="paper",
        ),
        path=p,
    )
    r = client_no_auth.get(
        "/assets/chart/trade-markers",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 200
    assert r.json()["count"] == 1
    assert r.json()["markers"][0]["side"] == "buy"


def test_trade_markers_rejects_bad_range(client_no_auth):
    r = client_no_auth.get(
        "/assets/chart/trade-markers",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-02T00:00:00Z",
            "end": "2026-04-01T00:00:00Z",
        },
    )
    assert r.status_code == 422
