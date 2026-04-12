"""FB-AP-020: universe API routes."""

from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime.alpaca_universe_store import replace_alpaca_universe_rows
from control_plane import api


@pytest.fixture
def client_universe(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> TestClient:
    db = tmp_path / "alpaca_u.sqlite"
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None, alpaca_universe_db_path=db))
    replace_alpaca_universe_rows(
        db,
        [
            {
                "canonical_symbol": "ETH-USD",
                "alpaca_symbol": "ETHUSD",
                "name": "Ethereum",
                "asset_class": "crypto",
                "exchange": "CRYPTO",
                "tradable": True,
                "raw_json": "{}",
            }
        ],
    )
    return TestClient(api.app)


def test_get_universe_alpaca(client_universe: TestClient) -> None:
    r = client_universe.get("/universe/alpaca")
    assert r.status_code == 200
    j = r.json()
    assert j["total"] == 1
    assert j["rows"][0]["canonical_symbol"] == "ETH-USD"


def test_get_universe_alpaca_filter(client_universe: TestClient) -> None:
    r = client_universe.get("/universe/alpaca", params={"q": "BTC"})
    assert r.status_code == 200
    assert r.json()["total"] == 0
