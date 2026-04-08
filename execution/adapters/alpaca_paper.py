"""Alpaca paper trading adapter — execution only; never used for market data."""

from __future__ import annotations

import asyncio
import logging
import random
from decimal import Decimal

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck, PositionSnapshot
from execution.alpaca_util import from_alpaca_crypto_symbol, safe_exc_message, to_alpaca_crypto_symbol
from execution.intent_gate import require_execution_allowed

logger = logging.getLogger(__name__)

# Transient Alpaca/network failures — bounded retries with jitter
_RETRYABLE_EXCEPTION_NAMES = frozenset(
    {
        "ConnectionError",
        "TimeoutError",
        "ReadTimeout",
        "ConnectTimeout",
        "HTTPError",
        "APIError",
    }
)


def _is_retryable(exc: BaseException) -> bool:
    name = type(exc).__name__
    if name in _RETRYABLE_EXCEPTION_NAMES:
        return True
    # alpaca-py often wraps remote errors
    if "429" in str(exc) or "503" in str(exc) or "502" in str(exc):
        return True
    return False


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

    async def _with_retries(self, fn, *, max_attempts: int = 4, base_delay_s: float = 0.5):
        last: BaseException | None = None
        for attempt in range(1, max_attempts + 1):
            try:
                return await asyncio.to_thread(fn)
            except BaseException as e:
                last = e
                if attempt >= max_attempts or not _is_retryable(e):
                    raise
                delay = base_delay_s * (2 ** (attempt - 1)) + random.uniform(0, 0.15)
                logger.warning(
                    "alpaca transient error (attempt %s/%s): %s — retry in %.2fs",
                    attempt,
                    max_attempts,
                    safe_exc_message(e),
                    delay,
                )
                await asyncio.sleep(delay)
        assert last is not None
        raise last

    async def submit_order(self, order: OrderIntent) -> OrderAck:
        require_execution_allowed(order, self._settings)
        from alpaca.trading.enums import OrderSide, TimeInForce
        from alpaca.trading.requests import MarketOrderRequest

        client = self._ensure_client()
        sym = to_alpaca_crypto_symbol(order.symbol)
        side = OrderSide.BUY if order.side.value == "buy" else OrderSide.SELL
        req = MarketOrderRequest(
            symbol=sym,
            qty=float(order.quantity),
            side=side,
            time_in_force=TimeInForce.GTC,
        )

        def _submit():
            return client.submit_order(req)

        order_resp = await self._with_retries(_submit)
        return OrderAck(
            adapter=self.name,
            order_id=str(order_resp.id),
            status=str(order_resp.status),
            raw={"id": str(order_resp.id), "symbol": sym},
        )

    async def cancel_order(self, order_id: str) -> bool:
        client = self._ensure_client()

        def _cancel():
            return client.cancel_order_by_id(order_id)

        await self._with_retries(_cancel)
        return True

    async def fetch_positions(self) -> list[PositionSnapshot]:
        client = self._ensure_client()

        def _list():
            return client.get_all_positions()

        positions = await self._with_retries(_list)
        out: list[PositionSnapshot] = []
        for p in positions:
            sym = getattr(p, "symbol", "")
            qty = Decimal(str(getattr(p, "qty", "0")))
            out.append(
                PositionSnapshot(
                    symbol=from_alpaca_crypto_symbol(sym),
                    quantity=qty,
                    avg_entry_price=Decimal(str(getattr(p, "avg_entry_price", "0") or "0")),
                    raw={"symbol": sym, "alpaca_symbol": sym},
                )
            )
        return out
