"""
FB-AP-042: focused integration coverage — manifest ↔ lifecycle, binding abstain, chart scope, merge idempotency.
"""

from __future__ import annotations

from datetime import UTC, datetime

import polars as pl
import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.contracts.asset_model_manifest import AssetModelManifest
from app.runtime import asset_lifecycle_state as lc
from app.runtime import asset_model_registry as reg
from control_plane import api
from data_plane.storage.merge_canonical_bars import merge_canonical_bars_frames
from decision_engine.pipeline import resolve_serving_paths


@pytest.fixture
def dirs(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path / "manifests")
    monkeypatch.setattr(lc, "_DEFAULT_DIR", tmp_path / "lifecycle")
    (tmp_path / "manifests").mkdir(parents=True, exist_ok=True)


@pytest.fixture
def client_with_key(dirs, monkeypatch):
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="k"))
    return TestClient(api.app)


def test_manifest_lifecycle_and_chart_symbol_scope(
    client_with_key: TestClient, monkeypatch: pytest.MonkeyPatch
) -> None:
    """PUT manifest → lifecycle initialized_not_active; chart GET returns scoped symbol; DELETE → uninitialized."""
    async def fake_chart(_settings, *, symbol, start, end, interval_seconds, limit):
        assert symbol == "BTC-USD"
        return {
            "symbol": "BTC-USD",
            "interval_seconds": interval_seconds or 60,
            "start": start.isoformat(),
            "end": end.isoformat(),
            "limit": limit,
            "count": 1,
            "bars": [
                {
                    "ts": "2026-04-01T00:00:00+00:00",
                    "symbol": "BTC-USD",
                    "interval_seconds": 60,
                    "open": 1.0,
                    "high": 2.0,
                    "low": 0.5,
                    "close": 1.5,
                    "volume": 10.0,
                    "source": "test",
                    "schema_version": 1,
                }
            ],
        }

    monkeypatch.setattr(api, "query_canonical_bars_for_chart", fake_chart)

    r = client_with_key.put(
        "/assets/models/BTC-USD",
        json={"canonical_symbol": "BTC-USD", "forecaster_torch_path": "/x.pt"},
        headers={"X-API-Key": "k"},
    )
    assert r.status_code == 200
    assert client_with_key.get("/assets/lifecycle/BTC-USD").json()["lifecycle_state"] == (
        "initialized_not_active"
    )

    cr = client_with_key.get(
        "/assets/chart/bars",
        params={
            "symbol": "BTC-USD",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
            "interval_seconds": 60,
        },
    )
    assert cr.status_code == 200
    body = cr.json()
    assert body["symbol"] == "BTC-USD"
    assert len(body["bars"]) == 1
    assert body["bars"][0]["symbol"] == "BTC-USD"

    client_with_key.delete("/assets/models/BTC-USD", headers={"X-API-Key": "k"})
    assert client_with_key.get("/assets/lifecycle/BTC-USD").json()["lifecycle_state"] == (
        "uninitialized"
    )


def test_multi_symbol_manifest_guard_abstains_without_per_symbol_manifest(tmp_path, monkeypatch) -> None:
    """FB-AP-003/004: wrong-symbol / missing-manifest must not silently use another symbol's weights."""
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    w = tmp_path / "btc.npz"
    w.write_bytes(b"x" * 64)
    reg.save_manifest(
        AssetModelManifest(canonical_symbol="BTC-USD", forecaster_weights_path=str(w))
    )
    settings = AppSettings(
        market_data_symbols=["BTC-USD", "ETH-USD"],
        models_forecaster_weights_path=str(w),
    )
    rp_eth = resolve_serving_paths("ETH-USD", settings)
    assert rp_eth.binding_abstain is True
    assert rp_eth.forecaster_weights_path is None


def _bar_row(ts: datetime, sym: str, interval: int, close: float) -> dict:
    return {
        "timestamp": ts,
        "symbol": sym,
        "interval_seconds": interval,
        "open": 1.0,
        "high": 1.0,
        "low": 1.0,
        "close": close,
        "volume": 1.0,
    }


def test_merge_three_frames_duplicate_timestamp_last_wins() -> None:
    """FB-AP-015: duplicate (symbol, ts, interval) — last frame's row wins."""
    t = datetime(2026, 1, 1, 12, 0, 0, tzinfo=UTC)
    a = pl.DataFrame([_bar_row(t, "BTC-USD", 60, 1.0)])
    b = pl.DataFrame([_bar_row(t, "BTC-USD", 60, 2.0)])
    c = pl.DataFrame([_bar_row(t, "BTC-USD", 60, 3.0)])
    out = merge_canonical_bars_frames(a, b, c)
    assert out.height == 1
    assert float(out["close"][0]) == 3.0
