"""Market data service scaffold wiring."""

from __future__ import annotations

import pytest

pytest.importorskip("fastapi")
from fastapi.testclient import TestClient

from services.market_data_service.wiring import create_app


def test_market_data_ingest_raw_tick_publishes(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NM_MESSAGING_BACKEND", "in_memory")
    app = create_app()
    client = TestClient(app)
    r = client.post(
        "/ingest/raw-tick",
        json={"symbol": "BTC-USD", "mid_price": 50_000.0, "spread_bps": 3.0},
    )
    assert r.status_code == 200
    assert r.json().get("published") is True
