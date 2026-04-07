"""Alpaca paper trading adapter — execution only; never used for market data."""

from __future__ import annotations

import logging
from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck, PositionSnapshot
from execution.intent_gate import require_execution_allowed

logger = logging.getLogger(__name__)


def _to_alpaca_crypto_symbol(product_id: str) -> str:
    """Map BTC-USD → BTCUSD for Alpaca crypto routing."""
    return product_id.replace("-", "")


class AlpacaPaperExecutionAdapter(ExecutionAdapter):
    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings
        self._client = None

    def _ensure_client(self):
        if self._client is not None:
            return self._client
        try:
            from alpaca.trading.client import TradingClient
        except ImportError as e:
            raise RuntimeError("Install alpaca-py for paper execution: pip install alpaca-py") from e
        key = self._settings.alpaca_api_key.get_secret_value() if self._settings.alpaca_api_key else None
        sec = (
            self._settings.alpaca_api_secret.get_secret_value()
            if self._settings.alpaca_api_secret
            else None
        )
        if not key or not sec:
            raise RuntimeError("Alpaca keys missing (NM_ALPACA_API_KEY / NM_ALPACA_API_SECRET)")
        self._client = TradingClient(key, sec, paper=True)
        return self._client

    @property
    def name(self) -> str:
        return "alpaca_paper"

    async def submit_order(self, order: OrderIntent) -> OrderAck:
        require_execution_allowed(order, self._settings)
        from alpaca.trading.enums import OrderSide, TimeInForce
        from alpaca.trading.requests import MarketOrderRequest

        client = self._ensure_client()
        sym = _to_alpaca_crypto_symbol(order.symbol)
        side = OrderSide.BUY if order.side.value == "buy" else OrderSide.SELL
        req = MarketOrderRequest(
            symbol=sym,
            qty=float(order.quantity),
            side=side,
            time_in_force=TimeInForce.GTC,
        )
        import asyncio

        order_resp = await asyncio.to_thread(client.submit_order, req)
        return OrderAck(
            adapter=self.name,
            order_id=str(order_resp.id),
            status=str(order_resp.status),
            raw={"id": str(order_resp.id), "symbol": sym},
        )

    async def cancel_order(self, order_id: str) -> bool:
        client = self._ensure_client()
        import asyncio

        await asyncio.to_thread(client.cancel_order_by_id, order_id)
        return True

    async def fetch_positions(self) -> list[PositionSnapshot]:
        client = self._ensure_client()
        import asyncio

        positions = await asyncio.to_thread(client.get_all_positions)
        out: list[PositionSnapshot] = []
        for p in positions:
            sym = getattr(p, "symbol", "")
            qty = Decimal(str(getattr(p, "qty", "0")))
            out.append(
                PositionSnapshot(
                    symbol=sym,
                    quantity=qty,
                    avg_entry_price=Decimal(str(getattr(p, "avg_entry_price", "0") or "0")),
                    raw={"symbol": sym},
                )
            )
        return out
