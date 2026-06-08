"""ExecutionService routes by per-symbol execution mode (FB-AP-030)."""

from __future__ import annotations

from decimal import Decimal

import pytest
from pydantic import SecretStr

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent, OrderSide, OrderType
from app.runtime.asset_execution_mode import write_mode_override
from execution import service as execution_service_mod
from execution.adapters.stub import StubExecutionAdapter
from execution.service import ExecutionService


@pytest.mark.asyncio
async def test_submit_uses_adapter_matching_per_symbol_mode(
    tmp_path, monkeypatch: pytest.MonkeyPatch
) -> None:
    secret = "s"
    monkeypatch.setenv("NM_ASSET_EXECUTION_MODE_DIR", str(tmp_path / "em"))
    settings = AppSettings(
        execution_mode="paper",
        allow_unsigned_execution=True,
        risk_signing_secret=SecretStr(secret),
    )
    write_mode_override("BTC-USD", "live")

    created: dict[str, StubExecutionAdapter] = {}

    def fake_create(s: AppSettings) -> StubExecutionAdapter:
        m = s.execution_mode
        if m not in created:
            created[m] = StubExecutionAdapter(s)
        return created[m]

    monkeypatch.setattr(execution_service_mod, "create_execution_adapter", fake_create)

    svc = ExecutionService(settings)

    i_paper = OrderIntent(
        symbol="ETH-USD",
        side=OrderSide.BUY,
        quantity=Decimal("1"),
        order_type=OrderType.MARKET,
    )
    i_live = OrderIntent(
        symbol="BTC-USD",
        side=OrderSide.BUY,
        quantity=Decimal("1"),
        order_type=OrderType.MARKET,
    )
    await svc.submit_order(i_paper)
    await svc.submit_order(i_live)

    assert "paper" in created and "live" in created
    assert created["paper"].submitted[0].symbol == "ETH-USD"
    assert created["live"].submitted[0].symbol == "BTC-USD"


@pytest.mark.asyncio
async def test_fixed_adapter_bypasses_per_symbol_routing(monkeypatch: pytest.MonkeyPatch) -> None:
    secret = "s"
    settings = AppSettings(
        execution_mode="paper",
        allow_unsigned_execution=True,
        risk_signing_secret=SecretStr(secret),
    )
    stub = StubExecutionAdapter(settings)
    svc = ExecutionService(settings, adapter=stub)
    i = OrderIntent(
        symbol="BTC-USD",
        side=OrderSide.BUY,
        quantity=Decimal("1"),
        order_type=OrderType.MARKET,
    )
    await svc.submit_order(i)
    assert len(stub.submitted) == 1
