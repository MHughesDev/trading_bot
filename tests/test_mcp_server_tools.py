"""Tests for the MCP tool registry/tools (transport-agnostic; no `mcp` package needed)."""

from __future__ import annotations

from typing import Any

import pytest

from mcp_server import build_registry
from mcp_server.registry import ToolRegistry, ToolSpec
from mcp_server.tools import build_default_tools


class FakeBackend:
    """Records calls and returns canned responses; implements PlatformBackend structurally."""

    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any]]] = []

    def system_status(self) -> dict[str, Any]:
        self.calls.append(("system_status", {}))
        return {"ok": True, "execution_mode": "paper"}

    def list_assets(self) -> dict[str, Any]:
        self.calls.append(("list_assets", {}))
        return {"assets": ["BTC-USD"]}

    def get_bars(self, symbol: str, *, limit: int = 200, interval_seconds=None) -> dict[str, Any]:
        self.calls.append(("get_bars", {"symbol": symbol, "limit": limit, "iv": interval_seconds}))
        return {"symbol": symbol, "bars": [], "count": 0}

    def get_positions(self) -> dict[str, Any]:
        self.calls.append(("get_positions", {}))
        return {"positions": []}

    def get_pnl(self, *, window: str = "day") -> dict[str, Any]:
        self.calls.append(("get_pnl", {"window": window}))
        return {"window": window, "realized": 0.0}

    def get_latest_decision(self) -> dict[str, Any]:
        self.calls.append(("get_latest_decision", {}))
        return {"decision_record": None}

    def place_order(self, **kwargs: Any) -> dict[str, Any]:
        self.calls.append(("place_order", kwargs))
        return {"submitted": True, **kwargs}

    def flatten(self, symbol: str) -> dict[str, Any]:
        self.calls.append(("flatten", {"symbol": symbol}))
        return {"symbol": symbol, "flatten": {"skipped": "flat"}}

    def set_lifecycle(self, symbol: str, action: str) -> dict[str, Any]:
        self.calls.append(("set_lifecycle", {"symbol": symbol, "action": action}))
        return {"symbol": symbol, "action": action}

    def set_execution_mode(self, symbol: str, mode: str) -> dict[str, Any]:
        self.calls.append(("set_execution_mode", {"symbol": symbol, "mode": mode}))
        return {"symbol": symbol, "mode": mode}


@pytest.fixture
def registry() -> ToolRegistry:
    return build_registry(FakeBackend())


def test_registry_exposes_expected_tools(registry: ToolRegistry) -> None:
    names = set(registry.tool_names())
    assert {
        "get_system_status",
        "list_watched_assets",
        "get_recent_bars",
        "get_positions",
        "get_pnl",
        "get_latest_decision",
        "place_order",
        "flatten_position",
        "set_asset_lifecycle",
        "set_execution_mode",
    } <= names


def test_act_tools_are_marked_mutating(registry: ToolRegistry) -> None:
    by_name = {s.name: s for s in registry.list_tools()}
    for read in ("get_system_status", "get_recent_bars", "get_pnl"):
        assert by_name[read].mutating is False
    for act in ("place_order", "flatten_position", "set_asset_lifecycle", "set_execution_mode"):
        assert by_name[act].mutating is True


def test_every_tool_has_object_schema(registry: ToolRegistry) -> None:
    for spec in registry.list_tools():
        assert spec.input_schema.get("type") == "object"
        assert isinstance(spec.description, str) and spec.description


def test_get_recent_bars_passes_args() -> None:
    backend = FakeBackend()
    reg = build_registry(backend)
    out = reg.call_tool("get_recent_bars", {"symbol": "ETH-USD", "limit": 50})
    assert out == {"symbol": "ETH-USD", "bars": [], "count": 0}
    assert ("get_bars", {"symbol": "ETH-USD", "limit": 50, "iv": None}) in backend.calls


def test_place_order_routes_to_backend() -> None:
    backend = FakeBackend()
    reg = build_registry(backend)
    out = reg.call_tool(
        "place_order",
        {"symbol": "BTC-USD", "side": "buy", "quantity": "0.01", "order_type": "market"},
    )
    assert out["submitted"] is True
    assert out["side"] == "buy"
    name, kwargs = backend.calls[-1]
    assert name == "place_order"
    assert kwargs["symbol"] == "BTC-USD" and kwargs["quantity"] == "0.01"


def test_place_order_missing_required_arg_is_error() -> None:
    reg = build_registry(FakeBackend())
    out = reg.call_tool("place_order", {"symbol": "BTC-USD", "side": "buy"})  # no quantity
    assert "error" in out
    assert "quantity" in out["error"]


def test_flatten_and_lifecycle_and_mode() -> None:
    backend = FakeBackend()
    reg = build_registry(backend)
    assert reg.call_tool("flatten_position", {"symbol": "BTC-USD"})["symbol"] == "BTC-USD"
    assert reg.call_tool("set_asset_lifecycle", {"symbol": "BTC-USD", "action": "start"})[
        "action"
    ] == "start"
    assert reg.call_tool("set_execution_mode", {"symbol": "BTC-USD", "mode": "live"})[
        "mode"
    ] == "live"


def test_unknown_tool_returns_error(registry: ToolRegistry) -> None:
    out = registry.call_tool("does_not_exist", {})
    assert "error" in out
    assert "available" in out


def test_handler_exception_is_captured() -> None:
    reg = ToolRegistry()

    def _boom(_a: dict[str, Any]) -> dict[str, Any]:
        raise RuntimeError("kaboom")

    reg.register(ToolSpec(name="boom", description="d", input_schema={"type": "object"}, handler=_boom))
    out = reg.call_tool("boom", {})
    assert out["error"] == "kaboom"
    assert out["tool"] == "boom"


def test_duplicate_registration_rejected() -> None:
    reg = ToolRegistry()
    specs = build_default_tools(FakeBackend())
    reg.register(specs[0])
    with pytest.raises(ValueError):
        reg.register(specs[0])
