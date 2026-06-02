"""The concrete tool set an AI agent is equipped with to operate the platform.

Read tools give the agent the context it needs to decide (status, assets, bars, positions,
PnL, the latest decision record). Act tools let it trade and manage assets — every act
routes through the control-plane endpoints, so the RiskEngine gates and risk-signing apply
identically to agent and human. The agent is *well-equipped but not omnipotent*: it can
buy, sell, flatten, and manage lifecycle, but cannot bypass risk.
"""

from __future__ import annotations

from typing import Any

from mcp_server.backend import PlatformBackend
from mcp_server.registry import ToolSpec

_SYMBOL = {"type": "string", "description": "Canonical symbol, e.g. BTC-USD"}


def _require(args: dict[str, Any], key: str) -> Any:
    if key not in args or args[key] in (None, ""):
        raise ValueError(f"missing required argument: {key}")
    return args[key]


def build_default_tools(backend: PlatformBackend) -> list[ToolSpec]:
    """Return the default read/act tool specs bound to ``backend``."""

    return [
        ToolSpec(
            name="get_system_status",
            description=(
                "Overall platform status: execution mode, system power/mode, per-asset "
                "lifecycle states, and health. Call this first to orient."
            ),
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.system_status(),
        ),
        ToolSpec(
            name="list_watched_assets",
            description="List assets that have been initialized (model manifests) on the platform.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.list_assets(),
        ),
        ToolSpec(
            name="get_recent_bars",
            description=(
                "Recent OHLCV minute bars for a symbol — the price context for a decision. "
                "Returns up to `limit` bars at `interval_seconds` (default 60s)."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200},
                    "interval_seconds": {"type": "integer", "description": "Bar width (default 60)"},
                },
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_bars(
                _require(a, "symbol"),
                limit=int(a.get("limit", 200)),
                interval_seconds=a.get("interval_seconds"),
            ),
        ),
        ToolSpec(
            name="get_positions",
            description="Current open positions from the configured execution venue.",
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_positions(),
        ),
        ToolSpec(
            name="get_pnl",
            description="Realized + unrealized P&L summary over a rolling window.",
            input_schema={
                "type": "object",
                "properties": {
                    "window": {
                        "type": "string",
                        "enum": ["hour", "day", "month", "year", "all"],
                        "default": "day",
                    }
                },
                "additionalProperties": False,
            },
            handler=lambda a: backend.get_pnl(window=str(a.get("window", "day"))),
        ),
        ToolSpec(
            name="get_latest_decision",
            description=(
                "The most recent canonical DecisionRecord from the live process (regime, "
                "forecast, route, risk, reason codes) — what the platform last concluded."
            ),
            input_schema={"type": "object", "properties": {}, "additionalProperties": False},
            handler=lambda _a: backend.get_latest_decision(),
        ),
        ToolSpec(
            name="place_order",
            description=(
                "Place a buy or sell order. quantity is absolute base units. order_type is "
                "'market', 'limit' (needs limit_price), 'stop' (needs stop_price), or 'stop_limit' "
                "(needs both). time_in_force is gtc/ioc/fok/gtd. Pass mid_price to enable the "
                "available-cash check. The order passes RiskEngine gates and is risk-signed; it "
                "may be blocked (see `blocked` reason codes in the result)."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "side": {"type": "string", "enum": ["buy", "sell"]},
                    "quantity": {"type": "string", "description": "Absolute quantity, e.g. '0.01'"},
                    "order_type": {
                        "type": "string",
                        "enum": ["market", "limit", "stop", "stop_limit"],
                        "default": "market",
                    },
                    "limit_price": {
                        "type": "string",
                        "description": "Required for limit and stop_limit orders",
                    },
                    "stop_price": {
                        "type": "string",
                        "description": "Required for stop and stop_limit orders",
                    },
                    "time_in_force": {
                        "type": "string",
                        "enum": ["gtc", "ioc", "fok", "gtd"],
                        "default": "gtc",
                    },
                    "mid_price": {"type": "number", "description": "Reference mark for cash gate"},
                },
                "required": ["symbol", "side", "quantity"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.place_order(
                symbol=_require(a, "symbol"),
                side=_require(a, "side"),
                quantity=str(_require(a, "quantity")),
                order_type=str(a.get("order_type", "market")),
                limit_price=a.get("limit_price"),
                stop_price=a.get("stop_price"),
                time_in_force=str(a.get("time_in_force", "gtc")),
                mid_price=a.get("mid_price"),
            ),
            mutating=True,
        ),
        ToolSpec(
            name="flatten_position",
            description="Market-close the entire open position for a symbol.",
            input_schema={
                "type": "object",
                "properties": {"symbol": _SYMBOL},
                "required": ["symbol"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.flatten(_require(a, "symbol")),
            mutating=True,
        ),
        ToolSpec(
            name="set_asset_lifecycle",
            description=(
                "Manage an asset's lifecycle: 'initialize' (set up models), 'start' (begin "
                "watching/trading), or 'stop' (stop and flatten)."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "action": {"type": "string", "enum": ["initialize", "start", "stop"]},
                },
                "required": ["symbol", "action"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_lifecycle(_require(a, "symbol"), _require(a, "action")),
            mutating=True,
        ),
        ToolSpec(
            name="set_execution_mode",
            description="Override per-asset execution mode: 'paper', 'live', or 'default'.",
            input_schema={
                "type": "object",
                "properties": {
                    "symbol": _SYMBOL,
                    "mode": {"type": "string", "enum": ["paper", "live", "default"]},
                },
                "required": ["symbol", "mode"],
                "additionalProperties": False,
            },
            handler=lambda a: backend.set_execution_mode(_require(a, "symbol"), _require(a, "mode")),
            mutating=True,
        ),
    ]
