"""Optional MCP/stdio entrypoint exposing the tool registry over the MCP protocol.

Run with::

    python -m mcp_server.server      # requires:  pip install -e ".[mcp]"

The transport-agnostic registry/tools/backend do the real work; this module only adapts
them to an MCP stdio server. The ``mcp`` package is optional — if it is missing the import
flag stays False and :func:`main` explains how to install it, so importing this module (or
the rest of the package) never hard-fails.
"""

from __future__ import annotations

import json
import os

from mcp_server import build_registry
from mcp_server.backend import HttpPlatformBackend
from mcp_server.registry import ToolRegistry

try:  # optional dependency — only needed for the actual MCP transport
    import mcp.types as mcp_types
    from mcp.server import Server
    from mcp.server.stdio import stdio_server

    _HAS_MCP = True
except Exception:  # pragma: no cover - exercised only when mcp is installed
    _HAS_MCP = False


def build_backend_from_env() -> HttpPlatformBackend:
    """Construct the HTTP backend from environment (platform URL + operator API key)."""
    base = os.getenv("NM_MCP_PLATFORM_URL", "http://127.0.0.1:8000")
    api_key = os.getenv("NM_CONTROL_PLANE_API_KEY") or None
    return HttpPlatformBackend(base, api_key=api_key)


def build_mcp_server(registry: ToolRegistry):  # pragma: no cover - requires mcp package
    """Wrap a :class:`ToolRegistry` into a low-level MCP ``Server``."""
    if not _HAS_MCP:
        raise RuntimeError('the `mcp` package is required: pip install -e ".[mcp]"')

    server = Server("trading-bot")

    @server.list_tools()
    async def _list_tools() -> list:
        return [
            mcp_types.Tool(name=s.name, description=s.description, inputSchema=s.input_schema)
            for s in registry.list_tools()
        ]

    @server.call_tool()
    async def _call_tool(name: str, arguments: dict | None) -> list:
        import anyio

        result = await anyio.to_thread.run_sync(
            lambda: registry.call_tool(name, arguments or {})
        )
        return [mcp_types.TextContent(type="text", text=json.dumps(result, default=str))]

    return server


async def _run_stdio() -> None:  # pragma: no cover - requires mcp package
    registry = build_registry(build_backend_from_env())
    server = build_mcp_server(registry)
    async with stdio_server() as (read, write):
        await server.run(read, write, server.create_initialization_options())


def main() -> int:
    if not _HAS_MCP:
        print(
            "The MCP transport requires the `mcp` package. Install with:\n"
            '  pip install -e ".[mcp]"\n'
            "The tool registry (mcp_server.registry/tools/backend) works without it."
        )
        return 1
    import anyio  # pragma: no cover

    anyio.run(_run_stdio)  # pragma: no cover
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
