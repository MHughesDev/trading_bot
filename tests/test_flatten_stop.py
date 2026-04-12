"""Per-asset stop flatten (FB-AP-032)."""

from __future__ import annotations

from decimal import Decimal

import pytest
from pydantic import SecretStr

from app.config.settings import AppSettings
from app.contracts.orders import OrderSide
from execution.adapters.base_adapter import OrderAck, PositionSnapshot
from execution.flatten_stop import flatten_symbol_position
from execution.service import ExecutionService


class _FakeAdapter:
    name = "fake"

    def __init__(self) -> None:
        self.positions: list[PositionSnapshot] = []
        self.submitted: list = []

    async def fetch_positions(self) -> list[PositionSnapshot]:
        return list(self.positions)

    async def submit_order(self, order):
        self.submitted.append(order)
        return OrderAck(adapter=self.name, order_id="1", status="accepted", raw={})

    async def cancel_order(self, order_id: str) -> bool:
        return True


@pytest.mark.asyncio
async def test_flatten_long_submits_sell() -> None:
    settings = AppSettings(allow_unsigned_execution=True)
    ad = _FakeAdapter()
    ad.positions = [
        PositionSnapshot(symbol="BTC-USD", quantity=Decimal("2.5"), raw={}),
    ]
    svc = ExecutionService(settings, adapter=ad)
    r = await flatten_symbol_position(settings, "BTC-USD", execution_service=svc)
    assert r["submitted"] is True
    assert r["lifecycle_continue"] is True
    assert len(ad.submitted) == 1
    assert ad.submitted[0].side == OrderSide.SELL
    assert ad.submitted[0].quantity == Decimal("2.5")


@pytest.mark.asyncio
async def test_flatten_short_submits_buy() -> None:
    settings = AppSettings(allow_unsigned_execution=True)
    ad = _FakeAdapter()
    ad.positions = [
        PositionSnapshot(symbol="ETH-USD", quantity=Decimal("-1"), raw={}),
    ]
    svc = ExecutionService(settings, adapter=ad)
    r = await flatten_symbol_position(settings, "ETH-USD", execution_service=svc)
    assert r["submitted"] is True
    assert ad.submitted[0].side == OrderSide.BUY
    assert ad.submitted[0].quantity == Decimal("1")


@pytest.mark.asyncio
async def test_flatten_skips_when_flat() -> None:
    settings = AppSettings(allow_unsigned_execution=True)
    ad = _FakeAdapter()
    svc = ExecutionService(settings, adapter=ad)
    r = await flatten_symbol_position(settings, "BTC-USD", execution_service=svc)
    assert r["skipped"] == "flat"
    assert r["lifecycle_continue"] is True
    assert ad.submitted == []


@pytest.mark.asyncio
async def test_flatten_signed_intent_when_secret_configured() -> None:
    secret = "test-secret-hmac"
    settings = AppSettings(
        allow_unsigned_execution=False,
        risk_signing_secret=SecretStr(secret),
    )
    ad = _FakeAdapter()
    ad.positions = [PositionSnapshot(symbol="BTC-USD", quantity=Decimal("1"), raw={})]
    svc = ExecutionService(settings, adapter=ad)
    r = await flatten_symbol_position(settings, "BTC-USD", execution_service=svc)
    assert r["submitted"] is True
    assert "risk_signature" in ad.submitted[0].metadata
