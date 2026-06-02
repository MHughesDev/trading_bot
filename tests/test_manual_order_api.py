"""Tests for the /trade/order and /trade/flatten control-plane endpoints."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from control_plane import api


@pytest.fixture
def client(monkeypatch: pytest.MonkeyPatch) -> TestClient:
    # No API key + session disabled => open mutate routes (matches existing test pattern).
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            control_plane_api_key=None, execution_mode="paper", auth_session_enabled=False
        ),
    )
    # Route execution to the in-process mock adapter (no network, accepts unsigned intents).
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "mock_alpaca_paper")
    return TestClient(api.app)


def test_trade_order_buy_submits(client: TestClient) -> None:
    r = client.post(
        "/trade/order",
        json={"symbol": "BTC-USD", "side": "buy", "quantity": "0.01", "order_type": "market"},
    )
    assert r.status_code == 200
    body = r.json()
    assert body["submitted"] is True
    assert body["symbol"] == "BTC-USD"
    assert body["side"] == "buy"
    assert body["acks"] and body["acks"][0]["status"] == "filled"


def test_trade_order_sell_submits(client: TestClient) -> None:
    r = client.post(
        "/trade/order",
        json={"symbol": "ETH-USD", "side": "sell", "quantity": "0.5"},
    )
    assert r.status_code == 200
    assert r.json()["submitted"] is True


def test_trade_order_invalid_side_not_submitted(client: TestClient) -> None:
    r = client.post(
        "/trade/order",
        json={"symbol": "BTC-USD", "side": "hodl", "quantity": "1"},
    )
    assert r.status_code == 200
    body = r.json()
    assert body["submitted"] is False
    assert "invalid_side" in body["blocked"]


def test_trade_order_rejects_nonpositive_quantity(client: TestClient) -> None:
    r = client.post(
        "/trade/order",
        json={"symbol": "BTC-USD", "side": "buy", "quantity": "0"},
    )
    # pydantic gt=0 validation => 422 before reaching the handler.
    assert r.status_code == 422


def test_trade_flatten_when_flat(client: TestClient) -> None:
    r = client.post("/trade/flatten", json={"symbol": "BTC-USD"})
    assert r.status_code == 200
    body = r.json()
    assert body["symbol"] == "BTC-USD"
    # Mock adapter has no positions => flatten reports "flat".
    assert body["flatten"]["skipped"] == "flat"


def test_trade_order_requires_key_when_configured(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "mock_alpaca_paper")
    c = TestClient(api.app)
    r = c.post("/trade/order", json={"symbol": "BTC-USD", "side": "buy", "quantity": "1"})
    assert r.status_code == 401
    r2 = c.post(
        "/trade/order",
        json={"symbol": "BTC-USD", "side": "buy", "quantity": "0.01"},
        headers={"X-API-Key": "secret-key"},
    )
    assert r2.status_code == 200
    assert r2.json()["submitted"] is True
