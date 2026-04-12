"""FB-AP-021: universe API routes."""

from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime.coinbase_universe_store import replace_coinbase_universe_rows
from control_plane import api


@pytest.fixture
def client_coinbase_u(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> TestClient:
    db = tmp_path / "cb_u.sqlite"
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None, coinbase_universe_db_path=db))
    replace_coinbase_universe_rows(
        db,
        [
            {
                "product_id": "ETH-USD",
                "base_name": "Ethereum",
                "quote_name": "US Dollar",
                "product_type": "SPOT",
                "trading_disabled": False,
                "is_disabled": False,
                "raw_json": "{}",
            }
        ],
    )
    return TestClient(api.app)


def test_get_universe_coinbase(client_coinbase_u: TestClient) -> None:
    r = client_coinbase_u.get("/universe/coinbase")
    assert r.status_code == 200
    j = r.json()
    assert j["total"] == 1
    assert j["rows"][0]["product_id"] == "ETH-USD"
