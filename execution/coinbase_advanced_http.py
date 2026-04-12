"""Async Coinbase Advanced Trade REST (CDP JWT) for orders, cancel, accounts."""

from __future__ import annotations

import json
import logging
from decimal import Decimal
from typing import Any
from urllib.parse import urlencode

import httpx
from coinbase import jwt_generator

from app.contracts.orders import OrderIntent, OrderSide, OrderType

logger = logging.getLogger(__name__)

BROKERAGE_BASE = "https://api.coinbase.com/api/v3/brokerage"


def _rest_jwt(method: str, path: str, api_key: str, api_secret: str) -> str:
    """Path must start with /api/v3/brokerage/..."""
    uri = jwt_generator.format_jwt_uri(method.upper(), path)
    return jwt_generator.build_rest_jwt(uri, api_key, api_secret)


class CoinbaseAdvancedHTTPClient:
    """Low-level async client; JWT is minted per request (short-lived)."""

    def __init__(
        self,
        api_key: str,
        api_secret: str,
        *,
        timeout_seconds: float = 30.0,
    ) -> None:
        self._api_key = api_key
        self._api_secret = api_secret
        self._client = httpx.AsyncClient(timeout=timeout_seconds)

    async def aclose(self) -> None:
        await self._client.aclose()

    async def _request(
        self,
        method: str,
        path: str,
        *,
        json_body: dict[str, Any] | None = None,
        params: dict[str, Any] | None = None,
    ) -> httpx.Response:
        if not path.startswith("/api/v3/brokerage"):
            path = "/api/v3/brokerage" + path
        jwt_path = path
        if params:
            qs = urlencode(sorted((str(k), str(v)) for k, v in params.items() if v is not None))
            jwt_path = f"{path}?{qs}"
        token = _rest_jwt(method, jwt_path, self._api_key, self._api_secret)
        headers = {
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
        }
        url = f"https://api.coinbase.com{path}"
        if method.upper() == "GET":
            return await self._client.get(url, headers=headers, params=params)
        if method.upper() == "POST":
            return await self._client.post(url, headers=headers, content=json.dumps(json_body or {}))
        if method.upper() == "DELETE":
            return await self._client.delete(url, headers=headers)
        raise ValueError(f"unsupported method {method}")

    async def create_order(self, order: OrderIntent) -> dict[str, Any]:
        """Submit a market order (IOC)."""
        path = "/api/v3/brokerage/orders"
        oid = order.client_order_id or f"tb-{order.symbol}-{id(order)}"
        side = "BUY" if order.side == OrderSide.BUY else "SELL"
        qty = str(order.quantity)
        body: dict[str, Any] = {
            "client_order_id": oid[:128],
            "product_id": order.symbol,
            "side": side,
            "order_configuration": {},
        }
        if order.order_type != OrderType.MARKET:
            raise NotImplementedError("Only market orders are implemented for Coinbase live V1")
        body["order_configuration"] = {
            "market_market_ioc": {
                "base_size": qty,
            }
        }
        resp = await self._request("POST", path, json_body=body)
        resp.raise_for_status()
        return resp.json()

    async def cancel_order(self, order_id: str) -> bool:
        path = f"/api/v3/brokerage/orders/{order_id}"
        resp = await self._request("DELETE", path)
        if resp.status_code == 404:
            return False
        resp.raise_for_status()
        return True

    async def list_accounts(self) -> list[dict[str, Any]]:
        path = "/api/v3/brokerage/accounts"
        resp = await self._request("GET", path)
        resp.raise_for_status()
        data = resp.json()
        return list(data.get("accounts", data) if isinstance(data, dict) else data)

    async def list_spot_products_paginated(
        self,
        *,
        limit: int = 250,
        product_type: str = "SPOT",
    ) -> list[dict[str, Any]]:
        """
        GET ``/products`` with pagination (cursor). FB-AP-021 — execution metadata only.

        See Coinbase Advanced Trade **List Products** (``product_type`` e.g. ``SPOT``).
        """
        out: list[dict[str, Any]] = []
        cursor: str | None = None
        path = "/api/v3/brokerage/products"
        for _page in range(500):
            params: dict[str, Any] = {
                "limit": max(1, min(int(limit), 1000)),
                "product_type": product_type,
            }
            if cursor:
                params["cursor"] = cursor
            resp = await self._request("GET", path, params=params)
            resp.raise_for_status()
            data = resp.json()
            if not isinstance(data, dict):
                break
            batch = data.get("products") or []
            for p in batch:
                if isinstance(p, dict):
                    out.append(p)
            nxt = data.get("cursor") or data.get("next_cursor")
            if not nxt:
                break
            nxt_s = str(nxt)
            if cursor is not None and nxt_s == cursor:
                break
            cursor = nxt_s
        return out


def accounts_to_position_snapshots(accounts: list[dict[str, Any]]) -> list[Any]:
    """Map brokerage accounts response to PositionSnapshot list (best-effort)."""
    from execution.adapters.base_adapter import PositionSnapshot

    out: list[PositionSnapshot] = []
    for acc in accounts:
        curr = (acc.get("currency") or acc.get("asset") or "").upper()
        if not curr or curr == "USD":
            continue
        avail = acc.get("available_balance") or {}
        if isinstance(avail, dict):
            val = avail.get("value", "0")
        else:
            val = str(avail)
        try:
            q = Decimal(str(val))
        except Exception:
            q = Decimal(0)
        if q == 0:
            continue
        sym = f"{curr}-USD"
        out.append(
            PositionSnapshot(
                symbol=sym,
                quantity=q,
                raw={"account": acc},
            )
        )
    return out


def order_id_from_create_response(data: dict[str, Any]) -> str:
    """Extract order id from POST /orders JSON (field names vary by API version)."""
    for key in ("order_id", "orderId", "success_response", "order"):
        v = data.get(key)
        if isinstance(v, str) and v:
            return v
        if isinstance(v, dict):
            oid = v.get("order_id") or v.get("orderId") or v.get("id")
            if oid:
                return str(oid)
    oid = data.get("id")
    return str(oid) if oid else ""
