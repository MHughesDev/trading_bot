"""GET /portfolio/positions with stub adapter."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client_stub_adapter(monkeypatch):
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "stub")
    monkeypatch.setattr(api, "settings", AppSettings(execution_mode="paper"))
    return TestClient(api.app)


def test_portfolio_positions_ok_stub(client_stub_adapter):
    r = client_stub_adapter.get("/portfolio/positions")
    assert r.status_code == 200
    body = r.json()
    assert body["ok"] is True
    assert body["positions"] == []
    assert body["adapter"] == "stub"
    assert body["execution_mode"] == "paper"
    assert body["mark_price_policy"]["source"] == "kraken_mid"
