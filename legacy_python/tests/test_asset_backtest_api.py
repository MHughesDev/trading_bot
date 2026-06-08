"""FB-AP-XXX: GET /strategies and POST /assets/backtest/{symbol}."""

from __future__ import annotations

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime import asset_model_registry as reg
from control_plane import api


_KEY = {"X-API-Key": "secret-key"}


@pytest.fixture
def client_no_auth(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key=None))
    return TestClient(api.app)


@pytest.fixture
def client_with_key(tmp_path, monkeypatch):
    monkeypatch.setattr(reg, "_DEFAULT_DIR", tmp_path)
    monkeypatch.setattr(api, "settings", AppSettings(control_plane_api_key="secret-key"))
    return TestClient(api.app)


def test_get_strategies_lists_ema_cross(client_no_auth):
    r = client_no_auth.get("/strategies")
    assert r.status_code == 200
    body = r.json()
    keys = {s["key"] for s in body["strategies"]}
    assert "ema_cross" in keys
    assert body["count"] == len(body["strategies"])
    ema = next(s for s in body["strategies"] if s["key"] == "ema_cross")
    assert ema["name"] == "EMA Cross"
    assert any(p["name"] == "fast_ema_period" for p in ema["params"])


def test_backtest_unknown_strategy_is_422(client_with_key, monkeypatch):
    r = client_with_key.post(
        "/assets/backtest/BTC-USD",
        headers=_KEY,
        json={
            "strategy_key": "does_not_exist",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 422


def test_backtest_no_bars_is_422(client_with_key, monkeypatch):
    async def fake_query(*_a, **_k):
        return {"symbol": "BTC-USD", "interval_seconds": 60, "count": 0, "bars": []}

    monkeypatch.setattr("control_plane.backtest_run.query_canonical_bars_for_chart", fake_query)
    r = client_with_key.post(
        "/assets/backtest/BTC-USD",
        headers=_KEY,
        json={
            "strategy_key": "ema_cross",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
        },
    )
    assert r.status_code == 422
    assert "no canonical bars" in r.json()["detail"]


def test_backtest_happy_path_returns_results(client_with_key, monkeypatch):
    bars = [
        {"ts": "2026-04-01T00:00:00+00:00", "open": 100.0, "high": 101.0, "low": 99.0, "close": 100.5, "volume": 10.0}
    ]

    async def fake_query(*_a, **_k):
        return {"symbol": "BTC-USD", "interval_seconds": 60, "count": len(bars), "bars": bars}

    captured = {}

    def fake_run_backtest(*, symbol, strategy_key, bars, strategy_params, interval_seconds, **kw):
        captured.update(
            symbol=symbol,
            strategy_key=strategy_key,
            n_bars=len(list(bars)),
            interval_seconds=interval_seconds,
            params=strategy_params,
        )

        class _R:
            def to_dict(self):
                return {"symbol": symbol, "strategy_key": strategy_key, "stats_pnls": {"USD": {"PnL (total)": 1.0}}}

        return _R()

    monkeypatch.setattr("control_plane.backtest_run.query_canonical_bars_for_chart", fake_query)
    monkeypatch.setattr("backtesting.nautilus_backtest.run_backtest", fake_run_backtest)

    r = client_with_key.post(
        "/assets/backtest/BTC-USD",
        headers=_KEY,
        json={
            "strategy_key": "ema_cross",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-04-02T00:00:00Z",
            "strategy_params": {"fast_ema_period": 5},
        },
    )
    assert r.status_code == 200
    out = r.json()
    assert out["stats_pnls"]["USD"]["PnL (total)"] == 1.0
    assert captured["symbol"] == "BTC-USD"
    assert captured["interval_seconds"] == 60
    assert captured["params"] == {"fast_ema_period": 5}


def test_backtest_engine_missing_is_503(client_with_key, monkeypatch):
    bars = [{"ts": "2026-04-01T00:00:00+00:00", "open": 1.0, "high": 1.0, "low": 1.0, "close": 1.0, "volume": 1.0}]

    async def fake_query(*_a, **_k):
        return {"symbol": "BTC-USD", "interval_seconds": 60, "count": 1, "bars": bars}

    def boom(**_kw):
        raise ImportError("nautilus_trader fork not installed")

    monkeypatch.setattr("control_plane.backtest_run.query_canonical_bars_for_chart", fake_query)
    monkeypatch.setattr("backtesting.nautilus_backtest.run_backtest", boom)

    r = client_with_key.post(
        "/assets/backtest/BTC-USD",
        headers=_KEY,
        json={"strategy_key": "ema_cross", "start": "2026-04-01T00:00:00Z", "end": "2026-04-02T00:00:00Z"},
    )
    assert r.status_code == 503
    assert r.json()["error"] == "backtest_engine_unavailable"
