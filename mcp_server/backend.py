"""Backend the MCP tools act on: the trading platform's control-plane API.

``PlatformBackend`` is the protocol the tools depend on (so tests can substitute a fake).
``HttpPlatformBackend`` implements it against the FastAPI control plane over HTTP, which
keeps the AI agent fully decoupled from the trading process — it is just another client of
the same endpoints humans use. Mutating calls send the operator ``X-API-Key`` (the MCP
server is automation, not a browser session).
"""

from __future__ import annotations

import datetime as _dt
from typing import Any, Protocol, runtime_checkable

import httpx


@runtime_checkable
class PlatformBackend(Protocol):
    """Capabilities the MCP tools require from the trading platform."""

    def system_status(self) -> dict[str, Any]: ...
    def list_assets(self) -> dict[str, Any]: ...
    def get_bars(
        self, symbol: str, *, limit: int = 200, interval_seconds: int | None = None
    ) -> dict[str, Any]: ...
    def get_positions(self) -> dict[str, Any]: ...
    def get_pnl(self, *, window: str = "day") -> dict[str, Any]: ...
    def get_latest_decision(self) -> dict[str, Any]: ...
    def place_order(
        self,
        *,
        symbol: str,
        side: str,
        quantity: str,
        order_type: str = "market",
        limit_price: str | None = None,
        stop_price: str | None = None,
        time_in_force: str = "gtc",
        mid_price: float | None = None,
    ) -> dict[str, Any]: ...
    def flatten(self, symbol: str) -> dict[str, Any]: ...
    def set_lifecycle(self, symbol: str, action: str) -> dict[str, Any]: ...
    def set_execution_mode(self, symbol: str, mode: str) -> dict[str, Any]: ...


class HttpPlatformBackend:
    """:class:`PlatformBackend` backed by the FastAPI control plane over HTTP."""

    def __init__(
        self,
        base_url: str = "http://127.0.0.1:8000",
        *,
        api_key: str | None = None,
        timeout: float = 15.0,
    ) -> None:
        self._base = base_url.rstrip("/")
        self._headers = {"X-API-Key": api_key} if api_key else {}
        self._timeout = timeout

    def _client(self) -> httpx.Client:
        return httpx.Client(base_url=self._base, headers=self._headers, timeout=self._timeout)

    def _get(self, path: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        with self._client() as c:
            r = c.get(path, params=params)
            r.raise_for_status()
            return r.json()

    def _post(self, path: str, json: dict[str, Any] | None = None) -> dict[str, Any]:
        with self._client() as c:
            r = c.post(path, json=json)
            r.raise_for_status()
            return r.json()

    def _put(self, path: str, json: dict[str, Any] | None = None) -> dict[str, Any]:
        with self._client() as c:
            r = c.put(path, json=json)
            r.raise_for_status()
            return r.json()

    def _delete(self, path: str) -> dict[str, Any]:
        with self._client() as c:
            r = c.delete(path)
            r.raise_for_status()
            return r.json()

    # --- reads ---
    def system_status(self) -> dict[str, Any]:
        return self._get("/status")

    def list_assets(self) -> dict[str, Any]:
        return self._get("/assets/models")

    def get_bars(
        self, symbol: str, *, limit: int = 200, interval_seconds: int | None = None
    ) -> dict[str, Any]:
        iv = interval_seconds or 60
        end = _dt.datetime.now(_dt.UTC)
        start = end - _dt.timedelta(seconds=iv * max(1, int(limit)))
        params: dict[str, Any] = {
            "symbol": symbol,
            "start": start.isoformat(),
            "end": end.isoformat(),
        }
        if interval_seconds:
            params["interval_seconds"] = int(interval_seconds)
        return self._get("/assets/chart/bars", params=params)

    def get_positions(self) -> dict[str, Any]:
        return self._get("/portfolio/positions")

    def get_pnl(self, *, window: str = "day") -> dict[str, Any]:
        return self._get("/pnl/summary", params={"range": window})

    def get_latest_decision(self) -> dict[str, Any]:
        return self._get("/governance/decision-record")

    # --- actions (mutating; carry operator key) ---
    def place_order(
        self,
        *,
        symbol: str,
        side: str,
        quantity: str,
        order_type: str = "market",
        limit_price: str | None = None,
        stop_price: str | None = None,
        time_in_force: str = "gtc",
        mid_price: float | None = None,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {
            "symbol": symbol,
            "side": side,
            "quantity": str(quantity),
            "order_type": order_type,
            "time_in_force": time_in_force,
        }
        if limit_price is not None:
            body["limit_price"] = str(limit_price)
        if stop_price is not None:
            body["stop_price"] = str(stop_price)
        if mid_price is not None:
            body["mid_price"] = float(mid_price)
        return self._post("/trade/order", json=body)

    def flatten(self, symbol: str) -> dict[str, Any]:
        return self._post("/trade/flatten", json={"symbol": symbol})

    def set_lifecycle(self, symbol: str, action: str) -> dict[str, Any]:
        a = action.strip().lower()
        if a == "initialize":
            return self._post(f"/assets/init/{symbol}")
        if a == "start":
            return self._post(f"/assets/lifecycle/{symbol}/start")
        if a == "stop":
            return self._post(f"/assets/lifecycle/{symbol}/stop")
        raise ValueError(f"invalid lifecycle action: {action!r} (use initialize|start|stop)")

    def set_execution_mode(self, symbol: str, mode: str) -> dict[str, Any]:
        m = mode.strip().lower()
        if m in ("default", "use default", ""):
            return self._delete(f"/assets/execution-mode/{symbol}")
        return self._put(f"/assets/execution-mode/{symbol}", json={"execution_mode": m})
