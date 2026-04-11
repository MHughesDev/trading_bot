"""GET /pnl/summary."""

from __future__ import annotations

from datetime import UTC, datetime
from decimal import Decimal
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

from app.config.settings import AppSettings
from execution.pnl_ledger import RealizedLedgerEntry, append_entry
from control_plane import api


@pytest.fixture
def client_pnl(monkeypatch, tmp_path: Path):
    ledger = tmp_path / "pnl_ledger.jsonl"

    def _path(repo_root=None):
        return ledger

    monkeypatch.setattr("execution.pnl_ledger.ledger_path", _path)
    monkeypatch.setenv("NM_EXECUTION_ADAPTER", "stub")
    monkeypatch.setattr(api, "settings", AppSettings(execution_mode="paper"))
    return TestClient(api.app), ledger


def test_pnl_summary_empty_ledger(client_pnl):
    client, _ledger = client_pnl
    r = client.get("/pnl/summary?range=day")
    assert r.status_code == 200
    body = r.json()
    assert body["realized_pnl_usd"] == "0"
    assert body["unrealized_pnl_usd"] == "0"
    assert body["positions_ok"] is True


def test_pnl_summary_with_realized(client_pnl):
    client, ledger = client_pnl
    append_entry(
        RealizedLedgerEntry(
            ts=datetime.now(tz=UTC),
            realized_pnl_usd=Decimal("42.5"),
            symbol="BTC-USD",
            source="test",
        ),
        path=ledger,
    )
    r = client.get("/pnl/summary?range=all")
    assert r.status_code == 200
    body = r.json()
    assert body["realized_pnl_usd"] == "42.5"
