"""Mark price policy and portfolio row enrichment (FB-DASH-04)."""

from __future__ import annotations

from decimal import Decimal
from unittest.mock import AsyncMock, MagicMock

import pytest

from app.config.settings import AppSettings
from app.contracts.portfolio import enrich_row_with_mark, position_snapshot_to_row
from execution.adapters.base_adapter import PositionSnapshot
from execution.mark_price import effective_mark_price_source, fetch_kraken_mid_prices
from execution.portfolio_positions import fetch_portfolio_positions


def test_effective_mark_price_source_defaults() -> None:
    paper = AppSettings(execution_mode="paper")
    live = AppSettings(execution_mode="live")
    assert effective_mark_price_source(paper) == "kraken_mid"
    assert effective_mark_price_source(live) == "venue_only"


def test_enrich_kraken_mid_computes_upnl() -> None:
    settings = AppSettings(execution_mode="paper", portfolio_mark_price_source_paper="kraken_mid")
    snap = PositionSnapshot(
        symbol="BTC-USD",
        quantity=Decimal("2"),
        avg_entry_price=Decimal("100"),
        unrealized_pnl=None,
    )
    row = position_snapshot_to_row(snap, venue_adapter="stub")
    mid = {"BTC-USD": Decimal("110")}
    out = enrich_row_with_mark(row, settings=settings, kraken_mid_by_symbol=mid)
    assert out.mark_price == "110"
    assert out.mark_price_source == "kraken_mid"
    assert out.unrealized_pnl == "20"


def test_enrich_venue_only_uses_venue_upnl() -> None:
    settings = AppSettings(execution_mode="live", portfolio_mark_price_source_live="venue_only")
    snap = PositionSnapshot(
        symbol="BTC-USD",
        quantity=Decimal("2"),
        avg_entry_price=Decimal("100"),
        unrealized_pnl=Decimal("20"),
    )
    row = position_snapshot_to_row(snap, venue_adapter="coinbase")
    out = enrich_row_with_mark(row, settings=settings, kraken_mid_by_symbol={})
    assert out.unrealized_pnl == "20"
    assert out.mark_price == "110"
    assert out.mark_price_source == "venue"


@pytest.mark.asyncio
async def test_fetch_portfolio_positions_includes_policy(monkeypatch) -> None:
    class FakeAdapter:
        name = "stub"

        async def fetch_positions(self) -> list[PositionSnapshot]:
            return [
                PositionSnapshot(
                    symbol="BTC-USD",
                    quantity=Decimal("1"),
                    avg_entry_price=Decimal("100"),
                    unrealized_pnl=None,
                )
            ]

    fake_svc = MagicMock()
    fake_svc.adapter = FakeAdapter()
    monkeypatch.setattr(
        "execution.portfolio_positions.ExecutionService",
        lambda _s: fake_svc,
    )
    monkeypatch.setattr(
        "execution.portfolio_positions.fetch_kraken_mid_prices",
        AsyncMock(return_value={"BTC-USD": Decimal("100")}),
    )
    settings = AppSettings(execution_mode="paper")
    out = await fetch_portfolio_positions(settings)
    assert out["ok"] is True
    assert out["mark_price_policy"]["source"] == "kraken_mid"
    pos = out["positions"][0]
    assert pos["mark_price_source"] == "kraken_mid"
    assert pos["mark_price"] == "100"


@pytest.mark.asyncio
async def test_fetch_kraken_mid_prices_uses_client(monkeypatch) -> None:
    c = MagicMock()
    c.ticker_mid = AsyncMock(side_effect=[73500.0, 2300.0])
    c.aclose = AsyncMock()
    out = await fetch_kraken_mid_prices(["BTC-USD", "ETH-USD"], client=c)
    assert "BTC-USD" in out and "ETH-USD" in out
    assert c.ticker_mid.await_count == 2
