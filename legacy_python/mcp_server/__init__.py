"""MCP server for the trading platform — the decoupled action surface for AI agents.

The trading platform is human-first: humans drive it through the Streamlit UI and the
FastAPI control plane. This package exposes that **same** control-plane surface as Model
Context Protocol (MCP) tools so an AI agent can read context (bars, positions, PnL, the
latest decision/forecast) and *act* (place orders, flatten, manage asset lifecycle) — all
through the platform's audited risk + execution path, never around it.

Layering:
- :mod:`mcp_server.registry`  — transport-agnostic tool registry (pure Python, testable).
- :mod:`mcp_server.backend`   — ``PlatformBackend`` protocol + an HTTP client of the API.
- :mod:`mcp_server.tools`     — the concrete read/act tool specs an agent is equipped with.
- :mod:`mcp_server.server`    — thin MCP/stdio wrapper (optional ``mcp`` dependency).

The AI models (forecaster/policy) are a *separate* system that merely lives in the same
repo; they never call execution directly. They act on the platform via these tools.
"""

from mcp_server.registry import ToolRegistry, ToolSpec
from mcp_server.tools import build_default_tools

__all__ = ["ToolRegistry", "ToolSpec", "build_default_tools", "build_registry"]


def build_registry(backend) -> ToolRegistry:
    """Convenience: a :class:`ToolRegistry` preloaded with the default platform tools."""
    registry = ToolRegistry()
    registry.register_all(build_default_tools(backend))
    return registry
