"""FB-AP-022: cross-venue platform-supported symbol set."""

from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from app.runtime.alpaca_universe_store import replace_alpaca_universe_rows
from app.runtime.coinbase_universe_store import replace_coinbase_universe_rows
from app.runtime.platform_supported_universe import (
    list_platform_supported_symbols,
    platform_supported_count,
    platform_supported_payload,
    platform_supported_status_summary,
)


def _seed_alpaca(db: Path) -> None:
    replace_alpaca_universe_rows(
        db,
        [
            {
                "canonical_symbol": "BTC-USD",
                "alpaca_symbol": "BTCUSD",
                "name": "Bitcoin",
                "asset_class": "crypto",
                "exchange": "CRYPTO",
                "tradable": True,
                "raw_json": "{}",
            },
            {
                "canonical_symbol": "SOL-USD",
                "alpaca_symbol": "SOLUSD",
                "name": "Solana",
                "asset_class": "crypto",
                "exchange": "CRYPTO",
                "tradable": True,
                "raw_json": "{}",
            },
        ],
        sync_error=None,
    )


def _seed_coinbase(db: Path) -> None:
    replace_coinbase_universe_rows(
        db,
        [
            {
                "product_id": "BTC-USD",
                "base_name": "BTC",
                "quote_name": "USD",
                "product_type": "SPOT",
                "trading_disabled": False,
                "is_disabled": False,
                "raw_json": "{}",
            },
            {
                "product_id": "ETH-USD",
                "base_name": "ETH",
                "quote_name": "USD",
                "product_type": "SPOT",
                "trading_disabled": False,
                "is_disabled": False,
                "raw_json": "{}",
            },
        ],
        sync_error=None,
    )


def test_intersection_count_and_list(tmp_path: Path) -> None:
    a = tmp_path / "a.sqlite"
    c = tmp_path / "c.sqlite"
    _seed_alpaca(a)
    _seed_coinbase(c)
    assert platform_supported_count(a, c, mode="intersection") == 1
    syms, total = list_platform_supported_symbols(a, c, mode="intersection", limit=10, offset=0)
    assert total == 1
    assert syms == ["BTC-USD"]


def test_union_includes_eth(tmp_path: Path) -> None:
    a = tmp_path / "a.sqlite"
    c = tmp_path / "c.sqlite"
    _seed_alpaca(a)
    _seed_coinbase(c)
    assert platform_supported_count(a, c, mode="union") == 3


def test_intersection_query_by_name(tmp_path: Path) -> None:
    a = tmp_path / "a.sqlite"
    c = tmp_path / "c.sqlite"
    _seed_alpaca(a)
    _seed_coinbase(c)
    syms, total = list_platform_supported_symbols(
        a, c, mode="intersection", limit=10, offset=0, query="bitcoin"
    )
    assert total == 1
    assert syms == ["BTC-USD"]


def test_payload_uses_settings_mode(tmp_path: Path) -> None:
    a = tmp_path / "a.sqlite"
    c = tmp_path / "c.sqlite"
    _seed_alpaca(a)
    _seed_coinbase(c)
    s = AppSettings(
        alpaca_universe_db_path=a,
        coinbase_universe_db_path=c,
        platform_supported_universe_mode="union",
    )
    p = platform_supported_payload(s, limit=50, offset=0)
    assert p["mode"] == "union"
    assert p["total"] == 3
    assert set(p["symbols"]) == {"BTC-USD", "SOL-USD", "ETH-USD"}


def test_status_summary() -> None:
    s = AppSettings()
    out = platform_supported_status_summary(s)
    assert "rule_version" in out
    assert out["mode"] == "intersection"


@pytest.fixture()
def client_ps(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> TestClient:
    from control_plane import api

    a = tmp_path / "a.sqlite"
    c = tmp_path / "c.sqlite"
    _seed_alpaca(a)
    _seed_coinbase(c)
    monkeypatch.setattr(
        api,
        "settings",
        AppSettings(
            alpaca_universe_db_path=a,
            coinbase_universe_db_path=c,
            platform_supported_universe_mode="intersection",
        ),
    )
    return TestClient(api.app)


def test_api_platform_supported(client_ps: TestClient) -> None:
    r = client_ps.get("/universe/platform-supported")
    assert r.status_code == 200
    j = r.json()
    assert j["ok"] is True
    assert j["total"] == 1
    assert j["symbols"] == ["BTC-USD"]


def test_status_includes_block(client_ps: TestClient) -> None:
    r = client_ps.get("/status")
    assert r.status_code == 200
    b = r.json()["platform_supported_universe"]
    assert b["symbol_count"] == 1
    assert b["mode"] == "intersection"
