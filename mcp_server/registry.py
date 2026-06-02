"""Transport-agnostic tool registry for the MCP server.

A :class:`ToolSpec` is a named, JSON-schema-described callable. The :class:`ToolRegistry`
holds specs and dispatches calls. This is deliberately independent of the ``mcp`` package
so it can be unit-tested without any transport, and reused by other front-ends (HTTP, CLI).
Handlers are synchronous and must return JSON-serializable dicts; exceptions are captured
into an ``{"error": ...}`` result so a misbehaving tool never crashes the agent session.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

ToolHandler = Callable[[dict[str, Any]], dict[str, Any]]


@dataclass(frozen=True)
class ToolSpec:
    """A single agent-callable tool: name, human description, JSON schema, handler."""

    name: str
    description: str
    input_schema: dict[str, Any]
    handler: ToolHandler
    mutating: bool = False  # True for tools that place/cancel orders or change state.


@dataclass
class ToolRegistry:
    """Holds tool specs and dispatches ``call_tool`` by name."""

    _tools: dict[str, ToolSpec] = field(default_factory=dict)

    def register(self, spec: ToolSpec) -> None:
        if spec.name in self._tools:
            raise ValueError(f"duplicate tool name: {spec.name}")
        self._tools[spec.name] = spec

    def register_all(self, specs: list[ToolSpec]) -> None:
        for spec in specs:
            self.register(spec)

    def list_tools(self) -> list[ToolSpec]:
        return list(self._tools.values())

    def tool_names(self) -> list[str]:
        return list(self._tools.keys())

    def get(self, name: str) -> ToolSpec | None:
        return self._tools.get(name)

    def call_tool(self, name: str, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
        spec = self._tools.get(name)
        if spec is None:
            return {"error": f"unknown tool: {name}", "available": self.tool_names()}
        try:
            result = spec.handler(arguments or {})
        except Exception as exc:  # never let a tool crash the agent loop
            return {"error": str(exc), "tool": name}
        if not isinstance(result, dict):
            return {"result": result}
        return result
