"""
Coinbase Advanced Trade live execution.

Uses CDP JWT (ECDSA) per request via `coinbase-advanced-py` + httpx async REST.
"""

from __future__ import annotations

import logging

from app.config.settings import AppSettings
from app.contracts.orders import OrderIntent
from execution.adapters.base_adapter import ExecutionAdapter, OrderAck, PositionSnapshot
from execution.coinbase_advanced_http import (
    CoinbaseAdvancedHTTPClient,
    accounts_to_position_snapshots,
    order_id_from_create_response,
)
from execution.intent_gate import require_execution_allowed
from observability.metrics import ORDER_FAIL, ORDER_SUCCESS

logger = logging.getLogger(__name__)


class CoinbaseExecutionAdapter(ExecutionAdapter):
    def __init__(self, settings: AppSettings) -> None:
        self._settings = settings
        self._http: CoinbaseAdvancedHTTPClient | None = None

    def _client(self) -> CoinbaseAdvancedHTTPClient:
        if self._http is None:
            if not self._settings.coinbase_api_key or not self._settings.coinbase_api_secret:
                raise RuntimeError(
                    "Coinbase API credentials missing (NM_COINBASE_API_KEY / NM_COINBASE_API_SECRET)"
                )
            key = self._settings.coinbase_api_key.get_secret_value()
            secret = self._settings.coinbase_api_secret.get_secret_value()
            self._http = CoinbaseAdvancedHTTPClient(key, secret)
        return self._http

    @property
    def name(self) -> str:
        return "coinbase"

    async def submit_order(self, order: OrderIntent) -> OrderAck:
        require_execution_allowed(order, self._settings)
        client = self._client()
        try:
            raw = await client.create_order(order)
            oid = order_id_from_create_response(raw) or "unknown"
            status = str(raw.get("status") or raw.get("success") or "submitted")
            ORDER_SUCCESS.labels(adapter=self.name).inc()
            return OrderAck(
                adapter=self.name,
                order_id=oid,
                status=status,
                raw=raw if isinstance(raw, dict) else {"response": raw},
            )
        except Exception:
            ORDER_FAIL.labels(adapter=self.name).inc()
            raise

    async def cancel_order(self, order_id: str) -> bool:
        client = self._client()
        return await client.cancel_order(order_id)

    async def fetch_positions(self) -> list[PositionSnapshot]:
        client = self._client()
        accounts = await client.list_accounts()
        return list(accounts_to_position_snapshots(accounts))
