from __future__ import annotations

import pytest

from app.contracts.common import ExecutionMode, OrderType, RouteId, Side
from app.contracts.decisions import OrderIntent
from execution.adapters.alpaca_paper_adapter import AlpacaPaperExecutionAdapter
from execution.adapters.coinbase_adapter import CoinbaseExecutionAdapter
from execution.router import ExecutionRouter


@pytest.mark.asyncio
async def test_execution_router_uses_paper_adapter_in_paper_mode() -> None:
    router = ExecutionRouter(
        mode=ExecutionMode.PAPER,
        coinbase_adapter=CoinbaseExecutionAdapter(api_key="x", api_secret="y"),
        alpaca_paper_adapter=AlpacaPaperExecutionAdapter(),
    )
    order = OrderIntent(
        symbol="BTC-USD",
        side=Side.BUY,
        quantity=0.01,
        order_type=OrderType.MARKET,
        route_id=RouteId.SCALPING,
        decision_id="trace-1",
    )
    report = await router.submit_order(order)
    assert report.adapter == "alpaca_paper"
    assert report.status == "filled"
